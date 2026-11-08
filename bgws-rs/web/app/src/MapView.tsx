// The map: MapLibre for the ground, deck.gl on top for everything the game
// draws.

import { useEffect, useRef } from "react";
import type { Layer } from "@deck.gl/core";
import { MapboxOverlay } from "@deck.gl/mapbox";
import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";

export type Basemap = "dark" | "streets" | "satellite";

const BASEMAPS: Record<Basemap, { tiles: string[]; attribution: string; paint?: Record<string, number> }> = {
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
    map.current = m;
    overlay.current = o;
    return () => {
      m.remove();
      map.current = null;
      overlay.current = null;
    };
  }, []);

  // The basemap: one raster source, swapped in place.
  useEffect(() => {
    const m = map.current;
    if (!m) return;
    const apply = () => {
      if (m.getLayer("base")) m.removeLayer("base");
      if (m.getSource("base")) m.removeSource("base");
      const b = BASEMAPS[props.basemap];
      m.addSource("base", { type: "raster", tiles: b.tiles, tileSize: 256, attribution: b.attribution, maxzoom: 19 });
      m.addLayer({ id: "base", type: "raster", source: "base", paint: b.paint ?? {} });
    };
    if (m.isStyleLoaded()) apply();
    else m.once("load", apply);
  }, [props.basemap]);

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
