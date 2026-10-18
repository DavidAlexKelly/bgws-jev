/**
 * MgrsOverlay — MGRS grid lines + coordinate display for MapLibre GL JS.
 *
 * Adds:
 *   1. MGRS grid lines as a GeoJSON source, regenerated on map move
 *   2. Grid zone / 100km square labels
 *   3. Live MGRS coordinate readout under the mouse cursor
 *
 * Grid detail scales with zoom:
 *   z6-z8:  100km squares (GZD + 100km ID, e.g. "33W WQ")
 *   z9-z11: 10km squares
 *   z12+:   1km squares
 *
 * Usage: call initMgrsOverlay(map) once you have the map (e.g. from
 * <DechoBasemap onMapReady>). Call removeMgrsOverlay(map) on cleanup.
 *
 * Moved over from the Mapplications repo with BGWS.
 */

import maplibregl from "maplibre-gl";

import { FONT_REGULAR, whenStyleLoaded } from "@acc/decho-basemap";
import { LatLon } from "geodesy/mgrs.js";
import Utm from "geodesy/utm.js";

// ── Grid interval by zoom ──────────────────────────────────────────────────

function gridIntervalMetres(zoom: number): number {
  if (zoom >= 12) return 1_000;      // 1km
  if (zoom >= 9)  return 10_000;     // 10km
  return                100_000;     // 100km
}

function labelPrecision(zoom: number): number {
  if (zoom >= 12) return 5;  // 1m  — full 10-digit ref
  if (zoom >= 9)  return 4;  // 10m — 8-digit ref
  if (zoom >= 7)  return 3;  // 100m — show 100km square ID
  return                2;   // GZD only
}

// ── Generate GeoJSON grid lines for current viewport ────────────────────────

function generateMgrsGrid(map: maplibregl.Map): GeoJSON.FeatureCollection {
  const zoom = map.getZoom();
  const bounds = map.getBounds();
  const interval = gridIntervalMetres(zoom);
  const features: GeoJSON.Feature[] = [];

  const sw = bounds.getSouthWest();
  const ne = bounds.getNorthEast();

  // Expand bounds slightly to avoid edge clipping
  const minLng = sw.lng - 0.5;
  const maxLng = ne.lng + 0.5;
  const minLat = Math.max(sw.lat - 0.5, -80);
  const maxLat = Math.min(ne.lat + 0.5, 84);

  // Sample UTM zones across the viewport
  // For each UTM zone that intersects the bbox, generate grid lines
  const minZone = Math.floor((minLng + 180) / 6) + 1;
  const maxZone = Math.floor((maxLng + 180) / 6) + 1;

  for (let zone = minZone; zone <= maxZone; zone++) {
    const hemisphere = minLat >= 0 ? "N" : "S";

    try {
      // SW corner in UTM
      const swUtm = new LatLon(Math.max(minLat, 0), Math.max(minLng, zone * 6 - 186))
        .toUtm();
      const neUtm = new LatLon(Math.min(maxLat, 84), Math.min(maxLng, zone * 6 - 180))
        .toUtm();

      const eMin = Math.floor(swUtm.easting / interval) * interval;
      const eMax = Math.ceil(neUtm.easting / interval) * interval;
      const nMin = Math.floor(Math.min(swUtm.northing, neUtm.northing) / interval) * interval;
      const nMax = Math.ceil(Math.max(swUtm.northing, neUtm.northing) / interval) * interval;

      // Vertical lines (easting)
      for (let e = eMin; e <= eMax; e += interval) {
        const coords: [number, number][] = [];
        for (let n = nMin; n <= nMax; n += interval / 4) {
          try {
            const utm = new Utm(zone, hemisphere, e, n);
            const ll = utm.toLatLon();
            if (ll.longitude >= minLng && ll.longitude <= maxLng &&
                ll.latitude >= minLat && ll.latitude <= maxLat) {
              coords.push([ll.longitude, ll.latitude]);
            }
          } catch { /* skip invalid */ }
        }
        if (coords.length >= 2) {
          features.push({
            type: "Feature",
            properties: { type: "grid-line" },
            geometry: { type: "LineString", coordinates: coords },
          });
        }
      }

      // Horizontal lines (northing)
      for (let n = nMin; n <= nMax; n += interval) {
        const coords: [number, number][] = [];
        for (let e = eMin; e <= eMax; e += interval / 4) {
          try {
            const utm = new Utm(zone, hemisphere, e, n);
            const ll = utm.toLatLon();
            if (ll.longitude >= minLng && ll.longitude <= maxLng &&
                ll.latitude >= minLat && ll.latitude <= maxLat) {
              coords.push([ll.longitude, ll.latitude]);
            }
          } catch { /* skip invalid */ }
        }
        if (coords.length >= 2) {
          features.push({
            type: "Feature",
            properties: { type: "grid-line" },
            geometry: { type: "LineString", coordinates: coords },
          });
        }
      }

      // Labels at grid intersections
      if (zoom >= 6) {
        const prec = labelPrecision(zoom);
        for (let e = eMin; e <= eMax; e += interval) {
          for (let n = nMin; n <= nMax; n += interval) {
            try {
              const utm = new Utm(zone, hemisphere, e, n);
              const ll = utm.toLatLon();
              if (ll.longitude >= minLng && ll.longitude <= maxLng &&
                  ll.latitude >= minLat && ll.latitude <= maxLat) {
                // eslint-disable-next-line @typescript-eslint/no-explicit-any
                const mgrs = (utm as any).toMgrs();
                const label = mgrs.toString(prec) as string;
                features.push({
                  type: "Feature",
                  properties: { type: "grid-label", label },
                  geometry: { type: "Point", coordinates: [ll.longitude, ll.latitude] },
                });
              }
            } catch { /* skip */ }
          }
        }
      }
    } catch { /* skip invalid zones */ }
  }

  return { type: "FeatureCollection", features };
}

// ── MapLibre layer setup ───────────────────────────────────────────────────

const GRID_SOURCE = "mgrs-grid";
const GRID_LINE_LAYER = "mgrs-grid-lines";
const GRID_LABEL_LAYER = "mgrs-grid-labels";

function addGridLayers(map: maplibregl.Map): void {
  if (map.getSource(GRID_SOURCE)) return; // already set up
  map.addSource(GRID_SOURCE, {
    type: "geojson",
    data: { type: "FeatureCollection", features: [] },
  });

  map.addLayer({
    id: GRID_LINE_LAYER,
    type: "line",
    source: GRID_SOURCE,
    filter: ["==", ["get", "type"], "grid-line"],
    paint: {
      "line-color": "#ff6600",
      "line-width": 0.5,
      "line-opacity": 0.6,
    },
  });

  map.addLayer({
    id: GRID_LABEL_LAYER,
    type: "symbol",
    source: GRID_SOURCE,
    filter: ["==", ["get", "type"], "grid-label"],
    layout: {
      "text-field": ["get", "label"],
      "text-size": 10,
      // Must match a font stack served by the Foundry map-assets dataset —
      // see @acc/decho-basemap. Without a `glyphs` key on the style
      // these labels render nothing while the grid lines still draw.
      "text-font": ["literal", FONT_REGULAR],
      "text-anchor": "bottom-left",
      "text-offset": [0.2, -0.2],
    },
    paint: {
      "text-color": "#ff6600",
      "text-halo-color": "rgba(0,0,0,0.7)",
      "text-halo-width": 1.5,
    },
  });
}

function updateGrid(map: maplibregl.Map): void {
  const source = map.getSource(GRID_SOURCE) as maplibregl.GeoJSONSource | undefined;
  if (!source) return;
  try {
    source.setData(generateMgrsGrid(map));
  } catch { /* ignore during transition */ }
}

// ── Public API ─────────────────────────────────────────────────────────────

let _gridEnabled = true;

export function initMgrsOverlay(map: maplibregl.Map): void {
  // Was `map.on("load", …)`. <DechoBasemap/> only hands the map over after
  // awaiting the style, by which time the one-shot "load" event has often
  // already fired — so the grid silently never appeared. whenStyleLoaded
  // resolves immediately in that case and waits correctly otherwise.
  void whenStyleLoaded(map).then(() => {
    try {
      addGridLayers(map);
      if (_gridEnabled) {
        updateGrid(map);
      } else {
        setGridEnabled(map, false);
      }
    } catch (err) {
      console.warn("[MgrsOverlay] could not add grid layers:", err);
    }
  });

  // Throttle grid updates to avoid thrashing during pan/zoom
  let _timeout: ReturnType<typeof setTimeout> | null = null;
  const throttledUpdate = () => {
    if (_timeout) clearTimeout(_timeout);
    _timeout = setTimeout(() => {
      if (_gridEnabled) updateGrid(map);
    }, 150);
  };

  map.on("moveend", throttledUpdate);
  map.on("zoomend", throttledUpdate);
}

export function removeMgrsOverlay(map: maplibregl.Map): void {
  if (map.getLayer(GRID_LABEL_LAYER)) map.removeLayer(GRID_LABEL_LAYER);
  if (map.getLayer(GRID_LINE_LAYER)) map.removeLayer(GRID_LINE_LAYER);
  if (map.getSource(GRID_SOURCE)) map.removeSource(GRID_SOURCE);
}

export function setGridEnabled(map: maplibregl.Map, enabled: boolean): void {
  _gridEnabled = enabled;
  const visibility = enabled ? "visible" : "none";
  if (map.getLayer(GRID_LINE_LAYER)) map.setLayoutProperty(GRID_LINE_LAYER, "visibility", visibility);
  if (map.getLayer(GRID_LABEL_LAYER)) map.setLayoutProperty(GRID_LABEL_LAYER, "visibility", visibility);
  if (enabled) updateGrid(map);
}

// ── MGRS coordinate from lngLat ────────────────────────────────────────────

export function lngLatToMgrs(lng: number, lat: number, precision = 5): string {
  try {
    const ll = new LatLon(lat, lng);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return (ll.toUtm() as any).toMgrs().toString(precision) as string;
  } catch {
    return `${lat.toFixed(5)}°N ${lng.toFixed(5)}°E`;
  }
}
