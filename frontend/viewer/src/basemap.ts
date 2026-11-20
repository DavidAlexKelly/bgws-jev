// The basemaps the viewer and the editor share: a pack's own OpenStreetMap
// layers (Protomaps' schema, from its basemap.pmtiles) and a world overview
// for low zooms from the Protomaps planet, which bgws-server proxies and
// caches at /world.pmtiles.

import type maplibregl from "maplibre-gl";

/** Land-use kinds by the texture they wear in quality mode (`textures.ts`). */
const LANDUSE_TEXTURES: [string[], string][] = [
  [["wood", "forest", "nature_reserve"], "tex-wood"],
  [["farmland", "farmyard", "orchard", "vineyard", "allotments"], "tex-farm"],
  [["grass", "grassland", "meadow", "park", "garden", "pitch", "golf_course", "cemetery", "recreation_ground", "village_green"], "tex-grass"],
  [["scrub", "heath"], "tex-scrub"],
  [["wetland", "marsh", "swamp", "bog", "reedbed"], "tex-wet"],
  [["residential", "suburb", "neighbourhood"], "tex-urban"],
  [["industrial", "commercial", "retail", "railway", "military", "parking"], "tex-industrial"],
  [["sand", "beach", "bare_rock", "quarry", "landfill"], "tex-sand"],
];

const WOODS = ["wood", "forest", "nature_reserve"];

/** A building's colour by its height (and a little per building, so a town is not one block). */
function buildingColour(light: boolean): maplibregl.ExpressionSpecification {
  const height = ["coalesce", ["get", "height"], ["get", "est_height"], 6] as maplibregl.ExpressionSpecification;
  // Over satellite imagery: a weathered concrete, about as dark as the imagery round it.
  const [low, mid, high] = light ? ["#57534c", "#635e56", "#716b62"] : ["#363f56", "#434d68", "#56617f"];
  const shade = ["%", ["coalesce", ["to-number", ["id"]], 0], 3] as maplibregl.ExpressionSpecification;
  return [
    "match",
    shade,
    0,
    ["interpolate", ["linear"], height, 4, low, 20, mid, 60, high],
    1,
    ["interpolate", ["linear"], height, 4, mid, 20, mid, 60, high],
    ["interpolate", ["linear"], height, 4, low, 20, low, 60, mid],
  ] as maplibregl.ExpressionSpecification;
}

/**
 * Buildings as blocks at their mapped height (else the pack's estimate from
 * the footprint, `est_height`, else 6 m), on
 * the 3D terrain. `light`: weathered concrete, for standing on satellite imagery.
 */
export function buildingBlocks(source: string, id: string, quality: boolean, light = false): maplibregl.LayerSpecification {
  return {
    id,
    type: "fill-extrusion",
    source,
    "source-layer": "buildings",
    minzoom: 13,
    paint: {
      "fill-extrusion-color": quality ? buildingColour(light) : light ? "#635e56" : "#3a4258",
      "fill-extrusion-height": ["coalesce", ["get", "height"], ["get", "est_height"], 6],
      "fill-extrusion-base": ["coalesce", ["get", "min_height"], 0],
      "fill-extrusion-opacity": quality ? 0.96 : 0.9,
      "fill-extrusion-vertical-gradient": true,
    },
  };
}

/**
 * Protomaps' OpenStreetMap layers, drawn dark like the screens around them.
 *
 * `quality` (the viewer's quality graphics) textures the ground by what it
 * is, gives water ripples and a shoreline, roads casings, buildings shading
 * by height and a cast shadow in 2D, and woods a canopy in 3D. It needs the
 * textures registered first (`addTextures`). Without it, flat colours: the
 * performance mode, and what other pages that share this get.
 */
export function terrainLayers(source = "base", dem = "base-dem", prefix = "base", threeD = false, quality = false): maplibregl.LayerSpecification[] {
  const kind = (k: string[]) => ["in", ["get", "kind"], ["literal", k]] as maplibregl.ExpressionSpecification;
  if (quality) return qualityLayers(source, dem, prefix, threeD, kind);
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
      ? buildingBlocks(source, `${prefix}-buildings`, false)
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

/** The quality mode's layers (see `terrainLayers`). */
function qualityLayers(source: string, dem: string, prefix: string, threeD: boolean, kind: (k: string[]) => maplibregl.ExpressionSpecification): maplibregl.LayerSpecification[] {
  const texture = ["match", ["get", "kind"], ...LANDUSE_TEXTURES.flatMap(([k, t]) => [k, t]), "tex-ground"] as unknown as maplibregl.ExpressionSpecification;
  const road = (id: string, kinds: string[], colour: string, casing: string, widths: number[], minzoom = 0): maplibregl.LayerSpecification[] => {
    const width = (k: number) => ["interpolate", ["linear"], ["zoom"], ...widths.flatMap((w, i) => [i === 0 ? 9 : 16, w * k])] as unknown as maplibregl.ExpressionSpecification;
    return [
      { id: `${prefix}-${id}-casing`, type: "line", source, "source-layer": "roads", minzoom, filter: kind(kinds), layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": casing, "line-width": width(1.9) } },
      { id: `${prefix}-${id}`, type: "line", source, "source-layer": "roads", minzoom, filter: kind(kinds), layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": colour, "line-width": width(1) } },
    ];
  };
  return [
    { id: `${prefix}-earth`, type: "fill", source, "source-layer": "earth", paint: { "fill-pattern": "tex-ground" } },
    { id: `${prefix}-landuse`, type: "fill", source, "source-layer": "landuse", paint: { "fill-pattern": texture } },
    { id: `${prefix}-water`, type: "fill", source, "source-layer": "water", filter: ["==", ["geometry-type"], "Polygon"], paint: { "fill-pattern": "tex-water" } },
    { id: `${prefix}-shore`, type: "line", source, "source-layer": "water", filter: ["==", ["geometry-type"], "Polygon"], paint: { "line-color": "#2b5a85", "line-width": ["interpolate", ["linear"], ["zoom"], 11, 0.4, 16, 1.5], "line-opacity": 0.7 } },
    { id: `${prefix}-waterway`, type: "line", source, "source-layer": "water", filter: ["==", ["geometry-type"], "LineString"], layout: { "line-cap": "round" }, paint: { "line-color": "#2b5f8f", "line-width": ["interpolate", ["linear"], ["zoom"], 10, 0.6, 15, 2.5] } },
    {
      id: `${prefix}-hillshade`,
      type: "hillshade",
      source: dem,
      paint: { "hillshade-shadow-color": "#02040a", "hillshade-highlight-color": "#56688c", "hillshade-accent-color": "#0a1020", "hillshade-exaggeration": 0.85, "hillshade-illumination-direction": 315 },
    },
    // Woods stand up as canopy in 3D, about 15 m tall.
    ...(threeD
      ? [
          {
            id: `${prefix}-canopy`,
            type: "fill-extrusion",
            source,
            "source-layer": "landuse",
            minzoom: 11,
            filter: kind(WOODS),
            paint: { "fill-extrusion-pattern": "tex-canopy", "fill-extrusion-height": 15, "fill-extrusion-opacity": 0.92, "fill-extrusion-vertical-gradient": true },
          } as maplibregl.LayerSpecification,
        ]
      : []),
    ...(threeD
      ? [buildingBlocks(source, `${prefix}-buildings`, true)]
      : ([
          // In 2D: a cast shadow to the south-east, then the roof, then its edge.
          { id: `${prefix}-buildings-shadow`, type: "fill", source, "source-layer": "buildings", minzoom: 13, paint: { "fill-color": "#04060c", "fill-opacity": 0.55, "fill-translate": [2, 2] } },
          { id: `${prefix}-buildings`, type: "fill", source, "source-layer": "buildings", minzoom: 13, paint: { "fill-color": buildingColour(false), "fill-opacity": 0.95 } },
          { id: `${prefix}-buildings-edge`, type: "line", source, "source-layer": "buildings", minzoom: 14, paint: { "line-color": "#5d688a", "line-width": 0.5, "line-opacity": 0.8 } },
        ] as maplibregl.LayerSpecification[])),
    // Railways as track: sleepers (short cross-ticks) under the rails.
    { id: `${prefix}-rail-ties`, type: "line", source, "source-layer": "roads", minzoom: 13, filter: kind(["rail"]), paint: { "line-color": "#6a6f82", "line-width": ["interpolate", ["linear"], ["zoom"], 13, 3, 17, 7], "line-dasharray": [0.25, 0.75], "line-opacity": 0.8 } },
    { id: `${prefix}-rail`, type: "line", source, "source-layer": "roads", filter: kind(["rail"]), paint: { "line-color": "#8a8fa2", "line-width": ["interpolate", ["linear"], ["zoom"], 10, 0.8, 17, 1.8] } },
    { id: `${prefix}-paths`, type: "line", source, "source-layer": "roads", minzoom: 13, filter: kind(["path"]), paint: { "line-color": "#3c4356", "line-width": 0.7, "line-dasharray": [2, 1.5] } },
    ...road("minor", ["minor_road", "other"], "#525a70", "#10141f", [0.4, 2.6], 11),
    ...road("major", ["major_road"], "#9a8a58", "#1a1408", [0.6, 4.2]),
    ...road("highway", ["highway"], "#cf9442", "#20140a", [0.8, 5.2]),
    { id: `${prefix}-bridges`, type: "line", source, "source-layer": "roads", filter: ["==", ["get", "is_bridge"], true], paint: { "line-color": "#d7dbe6", "line-width": ["interpolate", ["linear"], ["zoom"], 11, 1, 16, 4], "line-opacity": 0.75 } },
    { id: `${prefix}-boundaries`, type: "line", source, "source-layer": "boundaries", paint: { "line-color": "#6a7292", "line-width": 1, "line-dasharray": [4, 3] } },
  ];
}
