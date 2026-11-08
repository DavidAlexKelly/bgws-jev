// The map: MapLibre for the ground, deck.gl on top for everything the game
// draws. With a theatre pack, "terrain" draws the pack's own vector map and
// hillshade from its PMTiles, served by bgws-server: nothing from the internet.

import { useEffect, useRef } from "react";
import type { Layer } from "@deck.gl/core";
import { MapboxOverlay } from "@deck.gl/mapbox";
import maplibregl from "maplibre-gl";
import { Protocol } from "pmtiles";
import "maplibre-gl/dist/maplibre-gl.css";
import { terrainLayers, worldLayers } from "./basemap";
import { setTerrainZ, TERRAIN_EXAGGERATION } from "./layers";

export type Basemap = "terrain" | "dark" | "streets" | "satellite";

let pmtilesProtocol = false;
function registerPmtiles() {
  if (pmtilesProtocol) return;
  maplibregl.addProtocol("pmtiles", new Protocol().tile);
  pmtilesProtocol = true;
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
  /** Elevation as 3D terrain (the pack's own), with buildings as blocks. */
  threeD: boolean;
  /** The terrain under the overlay changed (3D tiles loaded, or the camera moved): rebuild its layers. */
  onLift?: () => void;
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
  const lift = useRef(props.onLift);
  lift.current = props.onLift;

  useEffect(() => {
    if (!el.current) return;
    const m = new maplibregl.Map({
      container: el.current,
      style: { version: 8, sources: {}, layers: [{ id: "bg", type: "background", paint: { "background-color": "#0b1220" } }] },
      center: [18.6, 54.2],
      zoom: 11,
      attributionControl: { compact: true },
    });
    m.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), "top-right");
    m.addControl(new maplibregl.ScaleControl({ unit: "metric" }), "top-right");
    const o = new MapboxOverlay({
      interleaved: false,
      // Drawn in layer order over the map: in 3D, heights relative to the view's
      // centre go below zero, and nothing the game draws should hide another.
      parameters: { depthCompare: "always", depthWriteEnabled: false },
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
    // In 3D the overlay's heights are relative to the view's centre: refresh them when the map settles.
    m.on("idle", () => {
      if (m.getTerrain()) lift.current?.();
    });
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
      if (m.getTerrain()) m.setTerrain(null);
      setTerrainZ(null);
      for (const l of m.getStyle().layers ?? []) if (l.id.startsWith("base") || l.id.startsWith("world")) m.removeLayer(l.id);
      for (const id of ["base", "base-dem", "base-terrain", "world"]) if (m.getSource(id)) m.removeSource(id);
      const flat = () => {
        if (m.getPitch() > 0) m.easeTo({ pitch: 0, bearing: 0, duration: 600 });
        lift.current?.();
      };
      if (props.basemap === "terrain") {
        registerPmtiles();
        const abs = (u: string) => `pmtiles://${location.origin}${u}`;
        // The world at low zooms (bgws-server proxies and caches it), the pack's own map over it.
        m.addSource("world", { type: "vector", url: abs("/world.pmtiles"), maxzoom: 9, attribution: "© OpenStreetMap contributors · Protomaps" });
        for (const l of worldLayers()) m.addLayer(l);
        if (!packBasemap) return flat();
        const solid = props.threeD && !!packDem;
        m.addSource("base", { type: "vector", url: abs(packBasemap), attribution: "© OpenStreetMap contributors · Protomaps" });
        if (packDem) m.addSource("base-dem", { type: "raster-dem", url: abs(packDem), encoding: "terrarium", tileSize: 512, attribution: "© Mapterhorn" });
        for (const l of terrainLayers("base", "base-dem", "base", solid)) if (l.id !== "base-hillshade" || packDem) m.addLayer(l);
        if (!solid) return flat();
        // The pack's own elevation, the same ground the game is played on.
        m.addSource("base-terrain", { type: "raster-dem", url: abs(packDem!), encoding: "terrarium", tileSize: 512 });
        m.setTerrain({ source: "base-terrain", exaggeration: TERRAIN_EXAGGERATION });
        setTerrainZ((lon, lat) => m.queryTerrainElevation([lon, lat]) ?? 0);
        if (m.getPitch() < 30) m.easeTo({ pitch: 55, duration: 800 });
        return;
      }
      flat();
      const b = BASEMAPS[props.basemap];
      m.addSource("base", { type: "raster", tiles: b.tiles, tileSize: 256, attribution: b.attribution, maxzoom: 19 });
      m.addLayer({ id: "base", type: "raster", source: "base", paint: b.paint ?? {} });
    };
    // The latest basemap wins: before the first load, only it is applied.
    if (loaded.current) apply();
    else pending.current = apply;
  }, [props.basemap, packBasemap, packDem, props.threeD]);

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
