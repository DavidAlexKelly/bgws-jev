//! Terrain from a partner's data (docs/design/INTEROP_PLAN.md §7): when a
//! federation agrees a common terrain source, `bgws terrain` builds from it
//! in place of our open sources where it is given, so every simulation
//! starts from the same ground.
//!
//! - **Elevation**: a GeoTIFF on a latitude/longitude grid, heights above
//!   mean sea level (or the WGS84 ellipsoid, taken off with the EGM96 geoid).
//!   Every terrain node it covers takes its height.
//! - **Land cover**: polygons (GeoJSON, or an ESRI shapefile), each with a
//!   class in one attribute, mapped to ours by a mapping file. Each node
//!   inside a polygon takes its class; later polygons win.
//! - **Roads**: lines (GeoJSON or shapefile), mapped to our road classes by
//!   the same file, added to ours or (with `replace`) in place of them.
//!
//! Coordinates must be longitude and latitude on WGS84 (EPSG:4326); other
//! projections, and GeoPackage files, convert with
//! `ogr2ogr -f GeoJSON -t_srs EPSG:4326 out.geojson in.gpkg`.
//!
//! The mapping file:
//! ```json
//! { "landCover": { "field": "class", "values": { "wood": "forestDense", "town": "urbanDense" } },
//!   "roads": { "field": "type", "values": { "A": "primary", "B": "secondary", "track": "track" } } }
//! ```
//! Land cover classes: open, crops, scrub, forestLight, forestDense,
//! urbanLight, urbanDense, water, wetland, bare. Road classes are OSM's
//! highway values (motorway, trunk, primary, secondary, tertiary,
//! unclassified, residential, service, track, path).

use anyhow::{anyhow, bail, Context, Result};
use bgws_env::LandCover;
use bgws_geodata::build::{Ground, RawRoad};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMap {
    pub field: String,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub land_cover: Option<FieldMap>,
    pub roads: Option<FieldMap>,
}

#[derive(Clone, Debug, Default)]
pub struct PartnerTerrain {
    pub elevation: Option<PathBuf>,
    /// The elevation's heights are above the ellipsoid.
    pub ellipsoidal: bool,
    pub land_cover: Option<PathBuf>,
    pub roads: Option<PathBuf>,
    /// The partner's roads only (else added to ours).
    pub replace_roads: bool,
    pub mapping: Mapping,
}

/// What a partner's data changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub heights: usize,
    pub land_cover: usize,
    pub unmapped_classes: Vec<String>,
    pub roads: usize,
}

/// A feature as read: its geometry (longitude, latitude) and its attributes.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// Rings: the first outer, the rest holes.
    Polygon(Vec<Vec<[f64; 2]>>),
    Line(Vec<[f64; 2]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub geometry: Geometry,
    pub attributes: BTreeMap<String, String>,
}

impl PartnerTerrain {
    pub fn is_empty(&self) -> bool {
        self.elevation.is_none() && self.land_cover.is_none() && self.roads.is_none()
    }

    pub fn with_mapping_file(mut self, path: Option<&Path>) -> Result<Self> {
        if let Some(p) = path {
            self.mapping = serde_json::from_str(&std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?).with_context(|| format!("{} is not a mapping file", p.display()))?;
        }
        Ok(self)
    }

    /// Change the ground with the partner's data.
    pub fn apply(&self, g: &mut Ground) -> Result<Applied> {
        let mut out = Applied::default();
        if let Some(p) = &self.elevation {
            let dem = crate::geotiff::Dem::read(p)?;
            out.heights = elevation(g, &|lat, lon| dem.height(lat, lon), self.ellipsoidal);
        }
        if let Some(p) = &self.land_cover {
            let map = self.mapping.land_cover.as_ref().ok_or_else(|| anyhow!("land cover from {} needs a mapping file with \"landCover\"", p.display()))?;
            let (n, unmapped) = land_cover(g, &read_features(p)?, map)?;
            out.land_cover = n;
            out.unmapped_classes = unmapped;
        }
        if let Some(p) = &self.roads {
            let map = self.mapping.roads.as_ref().ok_or_else(|| anyhow!("roads from {} need a mapping file with \"roads\"", p.display()))?;
            if self.replace_roads {
                g.roads.clear();
            }
            out.roads = roads(g, &read_features(p)?, map);
        }
        Ok(out)
    }
}

/// Node (i, j)'s latitude and longitude.
fn node_latlon(g: &Ground, spec: &bgws_env::GridSpec, i: usize, j: usize) -> (f64, f64) {
    let p = g.frame.enu_to_geodetic(bgws_core::Enu::new(spec.origin_e + i as f64 * spec.cell, spec.origin_n + j as f64 * spec.cell, 0.0));
    (p.lat, p.lon)
}

/// Each terrain node `height` covers takes its height; the count set.
pub fn elevation(g: &mut Ground, height: &dyn Fn(f64, f64) -> Option<f64>, ellipsoidal: bool) -> usize {
    let spec = g.terrain.elevation.spec;
    let geoid = bgws_interop::geoid::Geoid::egm96();
    let mut n = 0;
    for j in 0..spec.height {
        for i in 0..spec.width {
            let (lat, lon) = node_latlon(g, &spec, i, j);
            if let Some(h) = height(lat, lon) {
                let h = if ellipsoidal { geoid.to_msl(lat, lon, h) } else { h };
                g.terrain.elevation.set(i, j, h as f32);
                n += 1;
            }
        }
    }
    n
}

pub fn land_cover_class(name: &str) -> Option<LandCover> {
    Some(match name {
        "open" => LandCover::Open,
        "crops" => LandCover::Crops,
        "scrub" => LandCover::Scrub,
        "forestLight" => LandCover::ForestLight,
        "forestDense" => LandCover::ForestDense,
        "urbanLight" => LandCover::UrbanLight,
        "urbanDense" => LandCover::UrbanDense,
        "water" => LandCover::Water,
        "wetland" => LandCover::Wetland,
        "bare" => LandCover::Bare,
        _ => return None,
    })
}

fn inside(rings: &[Vec<[f64; 2]>], p: [f64; 2]) -> bool {
    // Even-odd over every ring: holes take themselves out.
    let mut c = false;
    for r in rings {
        let n = r.len();
        for k in 0..n {
            let (a, b) = (r[k], r[(k + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
                c = !c;
            }
        }
    }
    c
}

/// Paint each polygon's class over the nodes inside it; (nodes set, classes
/// the mapping does not name).
pub fn land_cover(g: &mut Ground, features: &[Feature], map: &FieldMap) -> Result<(usize, Vec<String>)> {
    let spec = g.terrain.land_cover.spec;
    let mut unmapped = std::collections::BTreeSet::new();
    let mut polys = vec![];
    for f in features {
        let Geometry::Polygon(rings) = &f.geometry else { continue };
        let Some(value) = f.attributes.get(&map.field) else { continue };
        let Some(name) = map.values.get(value) else {
            unmapped.insert(value.clone());
            continue;
        };
        let class = land_cover_class(name).ok_or_else(|| anyhow!("{name} is not a land cover class"))?;
        let pts = rings.iter().flatten();
        let bbox = pts.fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        polys.push((rings, class, bbox));
    }
    let mut n = 0;
    for j in 0..spec.height {
        for i in 0..spec.width {
            let (lat, lon) = node_latlon(g, &spec, i, j);
            let p = [lon, lat];
            if let Some((_, class, _)) = polys.iter().rev().find(|(r, _, b)| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3] && inside(r, p)) {
                g.terrain.land_cover.set(i, j, *class);
                n += 1;
            }
        }
    }
    Ok((n, unmapped.into_iter().collect()))
}

/// Add each mapped line as a road; the count added.
pub fn roads(g: &mut Ground, features: &[Feature], map: &FieldMap) -> usize {
    let ext = g.extent as f64;
    let mut n = 0;
    for f in features {
        let Geometry::Line(line) = &f.geometry else { continue };
        let Some(kind) = f.attributes.get(&map.field).and_then(|v| map.values.get(v)) else { continue };
        if line.len() < 2 {
            continue;
        }
        let pts = line
            .iter()
            .map(|p| {
                let (x, y) = bgws_geodata::mercator::tile_xy(p[0], p[1], g.zoom);
                [x * ext, y * ext]
            })
            .collect();
        // As the OSM build has them: the highway value as the detail, which
        // decides the engine's class, under Protomaps' broad kind.
        let group = match kind.as_str() {
            "motorway" => "highway",
            "trunk" | "primary" | "secondary" | "tertiary" => "major_road",
            "path" | "footway" | "cycleway" => "path",
            _ => "minor_road",
        };
        g.roads.push(RawRoad { kind: group.into(), detail: kind.clone(), bridge: false, tunnel: false, link: false, name: f.attributes.get("name").cloned(), line: pts });
        n += 1;
    }
    n
}

/// Read features from a GeoJSON file or an ESRI shapefile (by extension).
pub fn read_features(path: &Path) -> Result<Vec<Feature>> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("geojson") | Some("json") => geojson(&std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?),
        Some("shp") => shapefile(path),
        Some("gpkg") => bail!("{}: convert a GeoPackage first: ogr2ogr -f GeoJSON -t_srs EPSG:4326 out.geojson {}", path.display(), path.display()),
        _ => bail!("{}: not GeoJSON (.geojson, .json) or a shapefile (.shp)", path.display()),
    }
}

fn attr_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// GeoJSON: a FeatureCollection (or one Feature) of Polygon, MultiPolygon,
/// LineString and MultiLineString geometries.
pub fn geojson(text: &str) -> Result<Vec<Feature>> {
    let v: serde_json::Value = serde_json::from_str(text).context("not JSON")?;
    let features: Vec<&serde_json::Value> = match v["type"].as_str() {
        Some("FeatureCollection") => v["features"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
        Some("Feature") => vec![&v],
        other => bail!("not a GeoJSON feature collection ({other:?})"),
    };
    let pt = |p: &serde_json::Value| -> Option<[f64; 2]> { Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?]) };
    let line = |l: &serde_json::Value| -> Vec<[f64; 2]> { l.as_array().map(|a| a.iter().filter_map(pt).collect()).unwrap_or_default() };
    let rings = |r: &serde_json::Value| -> Vec<Vec<[f64; 2]>> { r.as_array().map(|a| a.iter().map(line).collect()).unwrap_or_default() };
    let mut out = vec![];
    for f in features {
        let attributes: BTreeMap<String, String> = f["properties"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), attr_string(v)?))).collect()).unwrap_or_default();
        let g = &f["geometry"];
        let c = &g["coordinates"];
        let geoms: Vec<Geometry> = match g["type"].as_str() {
            Some("Polygon") => vec![Geometry::Polygon(rings(c))],
            Some("MultiPolygon") => c.as_array().map(|a| a.iter().map(|p| Geometry::Polygon(rings(p))).collect()).unwrap_or_default(),
            Some("LineString") => vec![Geometry::Line(line(c))],
            Some("MultiLineString") => c.as_array().map(|a| a.iter().map(|l| Geometry::Line(line(l))).collect()).unwrap_or_default(),
            _ => vec![],
        };
        out.extend(geoms.into_iter().map(|geometry| Feature { geometry, attributes: attributes.clone() }));
    }
    Ok(out)
}

/// An ESRI shapefile (with its .dbf) of polygons or polylines.
pub fn shapefile(path: &Path) -> Result<Vec<Feature>> {
    use ::shapefile::dbase::FieldValue;
    use ::shapefile::Shape;
    let mut reader = ::shapefile::Reader::from_path(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let mut out = vec![];
    for item in reader.iter_shapes_and_records() {
        let (shape, record) = item.map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let mut attributes = BTreeMap::new();
        for (k, v) in record {
            let s = match v {
                FieldValue::Character(Some(s)) => s.trim().to_string(),
                FieldValue::Numeric(Some(n)) => n.to_string(),
                FieldValue::Integer(n) => n.to_string(),
                FieldValue::Float(Some(n)) => n.to_string(),
                FieldValue::Double(n) => n.to_string(),
                FieldValue::Logical(Some(b)) => b.to_string(),
                _ => continue,
            };
            attributes.insert(k, s);
        }
        let pts = |ps: &[::shapefile::Point]| ps.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>();
        let geoms: Vec<Geometry> = match shape {
            Shape::Polygon(p) => {
                // Each outer ring starts a polygon; inner rings are its holes.
                let mut polys: Vec<Vec<Vec<[f64; 2]>>> = vec![];
                for r in p.rings() {
                    match r {
                        ::shapefile::PolygonRing::Outer(v) => polys.push(vec![pts(v)]),
                        ::shapefile::PolygonRing::Inner(v) => {
                            if let Some(last) = polys.last_mut() {
                                last.push(pts(v));
                            }
                        }
                    }
                }
                polys.into_iter().map(Geometry::Polygon).collect()
            }
            Shape::Polyline(l) => l.parts().iter().map(|part| Geometry::Line(pts(part))).collect(),
            _ => vec![],
        };
        out.extend(geoms.into_iter().map(|geometry| Feature { geometry, attributes: attributes.clone() }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::{EnuFrame, Geodetic};

    /// 41 × 41 nodes at 50 m around 54.7 N, 20.5 E, flat at 10 m.
    fn ground() -> Ground {
        let frame = EnuFrame::new(Geodetic::new(54.7, 20.5, 0.0));
        let terrain = bgws_env::Terrain::flat(frame.origin, bgws_env::GridSpec::new(41, 41, 50.0, -1000.0, -1000.0), 10.0);
        Ground { frame, terrain, rivers: vec![], towns: vec![], roads: vec![], zoom: 14, extent: 4096, basemap: vec![], dem: vec![], planet: String::new(), stats: serde_json::Value::Null, surface: None }
    }

    fn lonlat(g: &Ground, e: f64, n: f64) -> [f64; 2] {
        let p = g.frame.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0));
        [p.lon, p.lat]
    }

    #[test]
    fn a_partner_elevation_replaces_ours_where_it_covers() {
        let mut g = ground();
        // The partner's ground: 50 m east of the origin, 5 m west of it; it
        // covers only the north half.
        let n = elevation(&mut g, &|lat, lon| (lat >= 54.7).then_some(if lon >= 20.5 { 50.0 } else { 5.0 }), false);
        assert!(n > 800 && n < 41 * 41, "{n} nodes covered");
        let h = |e: f64, nn: f64| g.terrain.elevation.nearest(e, nn);
        assert_eq!((h(500.0, 500.0), h(-500.0, 500.0), h(500.0, -500.0)), (50.0, 5.0, 10.0));
        // Ellipsoidal heights lose the geoid (about 29 m here).
        let mut g2 = ground();
        elevation(&mut g2, &|_, _| Some(50.0), true);
        let got = g2.terrain.elevation.nearest(0.0, 0.0) as f64;
        assert!((got - (50.0 - bgws_interop::geoid::Geoid::egm96().undulation(54.7, 20.5))).abs() < 0.01, "{got}");
    }

    #[test]
    fn partner_land_cover_and_roads_come_from_geojson() {
        let mut g = ground();
        // A wood with a clearing, and a road through it.
        let ring = |c: [f64; 2], r: f64| -> Vec<serde_json::Value> {
            [[-r, -r], [r, -r], [r, r], [-r, r], [-r, -r]]
                .iter()
                .map(|d| {
                    let p = lonlat(&g, c[0] + d[0], c[1] + d[1]);
                    serde_json::json!(p)
                })
                .collect()
        };
        let road: Vec<serde_json::Value> = [-900.0, 900.0].iter().map(|e| serde_json::json!(lonlat(&g, *e, 0.0))).collect();
        let text = serde_json::json!({ "type": "FeatureCollection", "features": [
            { "type": "Feature", "properties": { "class": "wood" }, "geometry": { "type": "Polygon", "coordinates": [ring([0.0, 0.0], 400.0), ring([0.0, 0.0], 100.0)] } },
            { "type": "Feature", "properties": { "class": "orchard" }, "geometry": { "type": "Polygon", "coordinates": [ring([700.0, 700.0], 100.0)] } },
            { "type": "Feature", "properties": { "type": "A", "name": "Partner road" }, "geometry": { "type": "LineString", "coordinates": road } }
        ]})
        .to_string();
        let features = geojson(&text).unwrap();
        assert_eq!(features.len(), 3);
        let map: Mapping = serde_json::from_value(serde_json::json!({ "landCover": { "field": "class", "values": { "wood": "forestDense" } }, "roads": { "field": "type", "values": { "A": "primary" } } })).unwrap();
        let (n, unmapped) = land_cover(&mut g, &features, map.land_cover.as_ref().unwrap()).unwrap();
        assert_eq!(unmapped, ["orchard"]);
        let at = |e: f64, nn: f64| g.terrain.land_cover.nearest(e, nn);
        assert_eq!(at(300.0, 0.0), LandCover::ForestDense, "in the wood");
        assert_eq!(at(0.0, 0.0), LandCover::Open, "in the clearing (a hole)");
        assert_eq!(at(800.0, -800.0), LandCover::Open, "outside");
        // 17 × 17 nodes in the wood less 5 × 5 in the clearing, give or take an edge.
        assert!((n as i64 - (17 * 17 - 5 * 5)).abs() <= 40, "{n}");
        assert_eq!(roads(&mut g, &features, map.roads.as_ref().unwrap()), 1);
        let r = &g.roads[0];
        assert_eq!((r.kind.as_str(), r.detail.as_str(), r.name.as_deref()), ("major_road", "primary", Some("Partner road")));
        assert_eq!(bgws_geodata::roads::engine_class(&r.kind, &r.detail), bgws_env::RoadClass::Primary);
        let (e, nn) = g.grid_to_enu(r.line[0]);
        assert!((e + 900.0).abs() < 1.0 && nn.abs() < 1.0, "the road starts where it should: {e:.1}, {nn:.1}");
    }

    #[test]
    fn partner_land_cover_comes_from_a_shapefile() {
        use ::shapefile::dbase::{FieldValue, Record, TableWriterBuilder};
        let mut g = ground();
        let dir = std::env::temp_dir().join(format!("bgws-shp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wood.shp");
        {
            let table = TableWriterBuilder::new().add_character_field("class".try_into().unwrap(), 20);
            let mut w = ::shapefile::Writer::from_path(&path, table).unwrap();
            // Clockwise, as shapefiles keep outer rings.
            let pts: Vec<::shapefile::Point> = [[-400.0, -400.0], [-400.0, 400.0], [400.0, 400.0], [400.0, -400.0], [-400.0, -400.0]]
                .iter()
                .map(|d| {
                    let p = lonlat(&g, d[0], d[1]);
                    ::shapefile::Point::new(p[0], p[1])
                })
                .collect();
            let poly = ::shapefile::Polygon::new(::shapefile::PolygonRing::Outer(pts));
            let mut rec = Record::default();
            rec.insert("class".to_string(), FieldValue::Character(Some("wood".into())));
            w.write_shape_and_record(&poly, &rec).unwrap();
        }
        let features = read_features(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].attributes.get("class").map(String::as_str), Some("wood"));
        let map = FieldMap { field: "class".into(), values: [("wood".to_string(), "forestLight".to_string())].into() };
        land_cover(&mut g, &features, &map).unwrap();
        assert_eq!(g.terrain.land_cover.nearest(0.0, 0.0), LandCover::ForestLight);
        assert_eq!(g.terrain.land_cover.nearest(800.0, 800.0), LandCover::Open);
    }
}
