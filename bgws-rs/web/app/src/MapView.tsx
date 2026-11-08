// The map: MapLibre for the ground, deck.gl on top for everything the game
// draws. With a theatre pack, "terrain" draws the pack's own vector map and
// hillshade from its PMTiles, served by bgws-server: nothing from the internet.

import { useEffect, useRef } from "react";
import type { Layer } from "@deck.gl/core";
import { MapboxOverlay } from "@deck.gl/mapbox";
import maplibregl from "maplibre-gl";
import { Protocol } from "pmtiles";
import "maplibre-gl/dist/maplibre-gl.css";

export type Basemap = "terrain" | "dark" | "streets" | "satellite";

let pmtilesProtocol = false;
function registerPmtiles() {
  if (pmtilesProtocol) return;
  maplibregl.addProtocol("pmtiles", new Protocol().tile);
  pmtilesProtocol = true;
}

/** Protomaps' OpenStreetMap layers, drawn dark like the screens around them. */
function terrainLayers(): maplibregl.LayerSpecification[] {
  const kind = (k: string[]) => ["in", ["get", "kind"], ["literal", k]] as maplibregl.ExpressionSpecification;
  return [
    { id: "base-earth", type: "fill", source: "base", "source-layer": "earth", paint: { "fill-color": "#141c2b" } },
    {
      id: "base-landuse",
      type: "fill",
      source: "base",
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
    { id: "base-water", type: "fill", source: "base", "source-layer": "water", filter: ["==", ["geometry-type"], "Polygon"], paint: { "fill-color": "#0e2740" } },
    { id: "base-waterway", type: "line", source: "base", "source-layer": "water", filter: ["==", ["geometry-type"], "LineString"], paint: { "line-color": "#1d4a72", "line-width": ["interpolate", ["linear"], ["zoom"], 10, 0.5, 15, 2] } },
    { id: "base-hillshade", type: "hillshade", source: "base-dem", paint: { "hillshade-shadow-color": "#05080f", "hillshade-highlight-color": "#3a4a66", "hillshade-exaggeration": 0.6 } },
    { id: "base-buildings", type: "fill", source: "base", "source-layer": "buildings", minzoom: 13, paint: { "fill-color": "#2a3044", "fill-opacity": 0.8 } },
    { id: "base-rail", type: "line", source: "base", "source-layer": "roads", filter: kind(["rail"]), paint: { "line-color": "#4a4f60", "line-width": 1, "line-dasharray": [3, 2] } },
    { id: "base-paths", type: "line", source: "base", "source-layer": "roads", minzoom: 13, filter: kind(["path"]), paint: { "line-color": "#353b4c", "line-width": 0.6 } },
    { id: "base-minor", type: "line", source: "base", "source-layer": "roads", filter: kind(["minor_road", "other"]), paint: { "line-color": "#454c60", "line-width": ["interpolate", ["linear"], ["zoom"], 11, 0.4, 16, 2.5] } },
    { id: "base-major", type: "line", source: "base", "source-layer": "roads", filter: kind(["major_road"]), paint: { "line-color": "#8b7d52", "line-width": ["interpolate", ["linear"], ["zoom"], 9, 0.6, 16, 4] } },
    { id: "base-highway", type: "line", source: "base", "source-layer": "roads", filter: kind(["highway"]), paint: { "line-color": "#c08a3a", "line-width": ["interpolate", ["linear"], ["zoom"], 8, 0.8, 16, 5] } },
    { id: "base-bridges", type: "line", source: "base", "source-layer": "roads", filter: ["==", ["get", "is_bridge"], true], paint: { "line-color": "#d7dbe6", "line-width": ["interpolate", ["linear"], ["zoom"], 11, 1, 16, 4], "line-opacity": 0.7 } },
    { id: "base-boundaries", type: "line", source: "base", "source-layer": "boundaries", paint: { "line-color": "#6a7292", "line-width": 1, "line-dasharray": [4, 3] } },
  ];
}

const BASEMAPS: Record<Exclude<Basemap, "terrain">, { tiles: string[]; attribution: string; paint?: Record<string, number> }> = {
  dark: {
    tiles: ["https://a.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png", "https://b.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png"],
    attribution: "© OpenStreetMap contributors © CARTO",
  },
  streets: { tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"], attribution: "© OpenStreetMap contributors", paint: { "raster-saturation": -0.4, "raster-brightness-max": 0.85 } },
  satellite: {
    tiles: ["https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"],
    attribution: "Imagery © Esri, Maxar, Earthstar Geographics",
    paint: { "raster-brightness-max": 0.8 },
  },
};

export function MapView(props: {
  layers: Layer[];
  basemap: Basemap;
  /** The pack's PMTiles (URLs under /packs/), for the terrain basemap. */
  pack: { basemap: string | null; dem: string | null } | null;
  /** The map's bounds as it moves: west, south, east, north. */
  onView?: (bbox: [number, number, number, number]) => void;
  /** Where to look first: the units' extent. */
  fit: [[number, number], [number, number]] | null;
  fitKey: string;
  /** Fly here when `key` changes (a click in the feed or the order of battle). */
  focus: { lon: number; lat: number; key: number } | null;
  onMapClick: (lon: number, lat: number) => void;
  tooltip: (info: { object?: unknown; layer?: { id: string } | null }) => string | null;
}) {
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<maplibregl.Map | null>(null);
  const overlay = useRef<MapboxOverlay | null>(null);
  const click = useRef(props.onMapClick);
  click.current = props.onMapClick;
  const tip = useRef(props.tooltip);
  tip.current = props.tooltip;
  const fitted = useRef("");
  // Set at the map's first load: isStyleLoaded() stays false while any tiles are loading.
  const loaded = useRef(false);
  const pending = useRef<(() => void) | null>(null);
  const view = useRef(props.onView);
  view.current = props.onView;

  useEffect(() => {
    if (!el.current) return;
    const m = new maplibregl.Map({
      container: el.current,
      style: { version: 8, sources: {}, layers: [{ id: "bg", type: "background", paint: { "background-color": "#0b1220" } }] },
      center: [18.6, 54.2],
      zoom: 11,
      attributionControl: { compact: true },
    });
    m.addControl(new maplibregl.NavigationControl({ visualizePitch: false }), "top-right");
    m.addControl(new maplibregl.ScaleControl({ unit: "metric" }), "top-right");
    const o = new MapboxOverlay({
      interleaved: false,
      layers: [],
      getTooltip: (info) => {
        const text = tip.current(info);
        return text ? { text, style: { background: "rgba(10,16,28,0.96)", color: "#e9ecfb", font: "11px/1.4 ui-monospace, monospace", border: "1px solid #2a3150", borderRadius: "3px", padding: "4px 6px" } } : null;
      },
    });
    m.addControl(o);
    m.on("click", (e) => click.current(e.lngLat.lng, e.lngLat.lat));
    const report = () => {
      const b = m.getBounds();
      view.current?.([b.getWest(), b.getSouth(), b.getEast(), b.getNorth()]);
    };
    m.on("moveend", report);
    m.once("load", () => {
      loaded.current = true;
      report();
      pending.current?.();
      pending.current = null;
    });
    map.current = m;
    overlay.current = o;
    return () => {
      m.remove();
      map.current = null;
      overlay.current = null;
    };
  }, []);

  // The basemap: swapped in place, under everything deck.gl draws.
  const packBasemap = props.pack?.basemap ?? null;
  const packDem = props.pack?.dem ?? null;
  useEffect(() => {
    const m = map.current;
    if (!m) return;
    const apply = () => {
      for (const l of m.getStyle().layers ?? []) if (l.id.startsWith("base")) m.removeLayer(l.id);
      for (const id of ["base", "base-dem"]) if (m.getSource(id)) m.removeSource(id);
      if (props.basemap === "terrain" && packBasemap) {
        registerPmtiles();
        const abs = (u: string) => `pmtiles://${location.origin}${u}`;
        m.addSource("base", { type: "vector", url: abs(packBasemap), attribution: "© OpenStreetMap contributors · Protomaps" });
        if (packDem) m.addSource("base-dem", { type: "raster-dem", url: abs(packDem), encoding: "terrarium", tileSize: 512, attribution: "© Mapterhorn" });
        for (const l of terrainLayers()) if (l.id !== "base-hillshade" || packDem) m.addLayer(l);
        return;
      }
      const b = BASEMAPS[props.basemap === "terrain" ? "dark" : props.basemap];
      m.addSource("base", { type: "raster", tiles: b.tiles, tileSize: 256, attribution: b.attribution, maxzoom: 19 });
      m.addLayer({ id: "base", type: "raster", source: "base", paint: b.paint ?? {} });
    };
    // The latest basemap wins: before the first load, only it is applied.
    if (loaded.current) apply();
    else pending.current = apply;
  }, [props.basemap, packBasemap, packDem]);

  useEffect(() => {
    overlay.current?.setProps({ layers: props.layers });
  }, [props.layers]);

  useEffect(() => {
    const m = map.current;
    if (!m || !props.fit || fitted.current === props.fitKey) return;
    fitted.current = props.fitKey;
    m.resize();
    m.fitBounds(props.fit, { padding: 60, maxZoom: 14, duration: 0 });
  }, [props.fit, props.fitKey]);

  useEffect(() => {
    const m = map.current;
    if (!m || !props.focus) return;
    m.flyTo({ center: [props.focus.lon, props.focus.lat], zoom: Math.max(m.getZoom(), 13), duration: 600 });
  }, [props.focus]);

  return <div ref={el} className="map" />;
}
