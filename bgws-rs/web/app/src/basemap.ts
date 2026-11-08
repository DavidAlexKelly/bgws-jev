// The basemaps the viewer and the editor share: a pack's own OpenStreetMap
// layers (Protomaps' schema, from its basemap.pmtiles) and a world overview
// for low zooms from the Protomaps planet, which bgws-server proxies and
// caches at /world.pmtiles.

import type maplibregl from "maplibre-gl";

/** Protomaps' OpenStreetMap layers, drawn dark like the screens around them. */
export function terrainLayers(source = "base", dem = "base-dem", prefix = "base", threeD = false): maplibregl.LayerSpecification[] {
  const kind = (k: string[]) => ["in", ["get", "kind"], ["literal", k]] as maplibregl.ExpressionSpecification;
  return [
    { id: `${prefix}-earth`, type: "fill", source, "source-layer": "earth", paint: { "fill-color": "#141c2b" } },
    {
      id: `${prefix}-landuse`,
      type: "fill",
      source,
      "source-layer": "landuse",
      paint: {
        "fill-color": [
          "match",
          ["get", "kind"],
          ["wood", "forest", "nature_reserve"],
          "#1b3224",
          ["farmland", "farmyard", "orchard", "vineyard", "allotments"],
          "#232a1c",
          ["grass", "grassland", "meadow", "park", "garden", "pitch", "golf_course", "cemetery", "recreation_ground", "village_green"],
          "#1b2820",
          ["scrub", "heath"],
          "#1e2a1e",
          ["wetland", "marsh", "swamp", "bog", "reedbed"],
          "#16292b",
          ["residential", "suburb", "neighbourhood"],
          "#1d2130",
          ["industrial", "commercial", "retail", "railway", "military", "parking"],
          "#202233",
          ["sand", "beach", "bare_rock", "quarry", "landfill"],
          "#2a2820",
          "#171f2d",
        ],
      },
    },
    { id: `${prefix}-water`, type: "fill", source, "source-layer": "water", filter: ["==", ["geometry-type"], "Polygon"], paint: { "fill-color": "#0e2740" } },
    { id: `${prefix}-waterway`, type: "line", source, "source-layer": "water", filter: ["==", ["geometry-type"], "LineString"], paint: { "line-color": "#1d4a72", "line-width": ["interpolate", ["linear"], ["zoom"], 10, 0.5, 15, 2] } },
    { id: `${prefix}-hillshade`, type: "hillshade", source: dem, paint: { "hillshade-shadow-color": "#05080f", "hillshade-highlight-color": "#3a4a66", "hillshade-exaggeration": 0.6 } },
    threeD
      ? {
          // Blocks at the mapped height (10 m where none is mapped), on the terrain.
          id: `${prefix}-buildings`,
          type: "fill-extrusion",
          source,
          "source-layer": "buildings",
          minzoom: 13,
          paint: {
            "fill-extrusion-color": "#3a4258",
            "fill-extrusion-height": ["coalesce", ["get", "height"], 10],
            "fill-extrusion-base": ["coalesce", ["get", "min_height"], 0],
            "fill-extrusion-opacity": 0.9,
          },
        }
      : { id: `${prefix}-buildings`, type: "fill", source, "source-layer": "buildings", minzoom: 13, paint: { "fill-color": "#2a3044", "fill-opacity": 0.8 } },
    { id: `${prefix}-rail`, type: "line", source, "source-layer": "roads", filter: kind(["rail"]), paint: { "line-color": "#4a4f60", "line-width": 1, "line-dasharray": [3, 2] } },
    { id: `${prefix}-paths`, type: "line", source, "source-layer": "roads", minzoom: 13, filter: kind(["path"]), paint: { "line-color": "#353b4c", "line-width": 0.6 } },
    { id: `${prefix}-minor`, type: "line", source, "source-layer": "roads", filter: kind(["minor_road", "other"]), paint: { "line-color": "#454c60", "line-width": ["interpolate", ["linear"], ["zoom"], 11, 0.4, 16, 2.5] } },
    { id: `${prefix}-major`, type: "line", source, "source-layer": "roads", filter: kind(["major_road"]), paint: { "line-color": "#8b7d52", "line-width": ["interpolate", ["linear"], ["zoom"], 9, 0.6, 16, 4] } },
    { id: `${prefix}-highway`, type: "line", source, "source-layer": "roads", filter: kind(["highway"]), paint: { "line-color": "#c08a3a", "line-width": ["interpolate", ["linear"], ["zoom"], 8, 0.8, 16, 5] } },
    { id: `${prefix}-bridges`, type: "line", source, "source-layer": "roads", filter: ["==", ["get", "is_bridge"], true], paint: { "line-color": "#d7dbe6", "line-width": ["interpolate", ["linear"], ["zoom"], 11, 1, 16, 4], "line-opacity": 0.7 } },
    { id: `${prefix}-boundaries`, type: "line", source, "source-layer": "boundaries", paint: { "line-color": "#6a7292", "line-width": 1, "line-dasharray": [4, 3] } },
  ];
}


/** The world overview: coastlines, land cover, water, borders and main roads
 *  from the planet's low zooms, under any pack. Place names are drawn by the
 *  page (no glyph server). */
export function worldLayers(source = "world"): maplibregl.LayerSpecification[] {
  const kind = (k: string[]) => ["in", ["get", "kind"], ["literal", k]] as maplibregl.ExpressionSpecification;
  return [
    { id: "world-earth", type: "fill", source, "source-layer": "earth", paint: { "fill-color": "#1a2333" } },
    {
      id: "world-landcover",
      type: "fill",
      source,
      "source-layer": "landcover",
      paint: {
        "fill-color": ["match", ["get", "kind"], "forest", "#1b3224", "farmland", "#222a1d", "grassland", "#1d2a22", "scrub", "#1e2a1e", "barren", "#2a2820", "urban_area", "#262a3a", "glacier", "#2c3442", "#1a2333"],
        "fill-opacity": 0.9,
      },
    },
    { id: "world-landuse", type: "fill", source, "source-layer": "landuse", filter: kind(["residential", "industrial", "commercial", "military"]), paint: { "fill-color": "#262a3a" } },
    { id: "world-water", type: "fill", source, "source-layer": "water", filter: ["==", ["geometry-type"], "Polygon"], paint: { "fill-color": "#0e2740" } },
    { id: "world-rivers", type: "line", source, "source-layer": "water", filter: ["==", ["geometry-type"], "LineString"], paint: { "line-color": "#1d4a72", "line-width": 0.8 } },
    { id: "world-major", type: "line", source, "source-layer": "roads", filter: kind(["major_road"]), minzoom: 7, paint: { "line-color": "#6e6447", "line-width": ["interpolate", ["linear"], ["zoom"], 7, 0.4, 12, 1.5] } },
    { id: "world-highway", type: "line", source, "source-layer": "roads", filter: kind(["highway"]), paint: { "line-color": "#a7793a", "line-width": ["interpolate", ["linear"], ["zoom"], 5, 0.5, 12, 2.5] } },
    { id: "world-borders", type: "line", source, "source-layer": "boundaries", paint: { "line-color": "#8a93b5", "line-width": ["interpolate", ["linear"], ["zoom"], 2, 0.6, 10, 1.5], "line-dasharray": [4, 2] } },
  ];
}
