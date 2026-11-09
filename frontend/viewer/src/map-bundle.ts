// The map for pages outside the React viewer (the scenario editor): MapLibre,
// the PMTiles protocol and the shared basemap styles, as window.BgwsMap.
// Built to dist/map.js and dist/map.css by vite.map.config.ts.

import maplibregl from "maplibre-gl";
import { Protocol } from "pmtiles";
import "maplibre-gl/dist/maplibre-gl.css";
import { terrainLayers, worldLayers } from "./basemap";

maplibregl.addProtocol("pmtiles", new Protocol().tile);
(window as unknown as { BgwsMap: unknown }).BgwsMap = { maplibregl, terrainLayers, worldLayers };
