/* ------------------------------------------------------------------ */
/*  Shared types for terrain-aware routing & reachability system      */
/* ------------------------------------------------------------------ */

// ── Inline GeoJSON helpers (no @types/geojson dependency) ──────────

export type GeoJsonPolygonGeometry = {
  type: "Polygon";
  coordinates: [number, number][][];
};

export type GeoJsonMultiPolygonGeometry = {
  type: "MultiPolygon";
  coordinates: [number, number][][][];
};

export type GeoJsonFeature = {
  type: "Feature";
  properties: Record<string, unknown>;
  geometry: GeoJsonPolygonGeometry | GeoJsonMultiPolygonGeometry;
};

// ── Terrain & mobility ─────────────────────────────────────────────

export enum TerrainFlag {
  Open = 0,
  Road = 1,
  Forest = 2,
  Marsh = 3,
  WaterCrossing = 4,  // shallow ford / bridge — navigable but very slow
  // 255 = deep water / impassable — no entry in TERRAIN_SPEEDS
}

export type MobilityClass = "tracked" | "wheeled" | "air" | "foot";

/**
 * Speed in km/h for each mobility class on each terrain type.
 * A value of 0 means the terrain is impassable for that class.
 * Terrain flag 255 (deep water) has no entry → treated as impassable.
 */
export const TERRAIN_SPEEDS: Record<MobilityClass, Record<number, number>> = {
  //                           open  road  forest  marsh  water-crossing
  tracked: {
    [TerrainFlag.Open]: 20,
    [TerrainFlag.Road]: 45,
    [TerrainFlag.Forest]: 4,
    [TerrainFlag.Marsh]: 2,
    [TerrainFlag.WaterCrossing]: 3,
  },
  wheeled: {
    [TerrainFlag.Open]: 25,
    [TerrainFlag.Road]: 65,
    [TerrainFlag.Forest]: 3,
    [TerrainFlag.Marsh]: 0,           // impassable
    [TerrainFlag.WaterCrossing]: 0,   // impassable (no ford for wheeled)
  },
  air: {
    [TerrainFlag.Open]: 800,
    [TerrainFlag.Road]: 800,
    [TerrainFlag.Forest]: 800,
    [TerrainFlag.Marsh]: 800,
    [TerrainFlag.WaterCrossing]: 800,
  },
  foot: {
    [TerrainFlag.Open]: 5,
    [TerrainFlag.Road]: 8,
    [TerrainFlag.Forest]: 2,
    [TerrainFlag.Marsh]: 0.5,
    [TerrainFlag.WaterCrossing]: 1,
  },
};

// ── Route & reachability results ───────────────────────────────────

export interface OSRMRoute {
  waypoints: [number, number][];
  distanceKm: number;
  durationSec: number;
}

export interface ReachableNode {
  lat: number;
  lon: number;
  costSec: number;
  globalNodeId: number;
}

export interface IsochroneResult {
  reachableNodes: ReachableNode[];
  envelope: GeoJsonFeature | null;
  /** Polygon covering impassable areas (water/blocked) within the envelope bounds */
  impassableWithinEnvelope: GeoJsonFeature | null;
  contours: GeoJsonFeature[];
  sourceLat: number;
  sourceLon: number;
  timeBudgetSec: number;
  mobilityClass: MobilityClass;
}

// ── Worker message types ───────────────────────────────────────────

// --- Requests (main thread → worker) ---

export type RouterWorkerRequest =
  | {
      type: "addTile";
      tileKey: string;
      nodesBuffer: ArrayBuffer;
      edgesBuffer: ArrayBuffer;
    }
  | {
      type: "route";
      id: string;
      fromLat: number;
      fromLon: number;
      toLat: number;
      toLon: number;
      mobilityClass: MobilityClass;
    }
  | {
      type: "isochrone";
      id: string;
      fromLat: number;
      fromLon: number;
      mobilityClass: MobilityClass;
      timeBudgetSec: number;
      contourIntervalsSec: number[];
    };

// --- Responses (worker → main thread) ---

export type RouterWorkerResponse =
  | {
      type: "routeResult";
      id: string;
      route: OSRMRoute | null;
    }
  | {
      type: "isochroneResult";
      id: string;
      nodes: ReachableNode[];
      envelopeCoords: [number, number][] | null;
      contourCoords: [number, number][][];
    }
  | {
      type: "ready";
    }
  | {
      type: "tilesLoaded";
      keys: string[];
    };

// ── Tile manifest ──────────────────────────────────────────────────

export interface TileManifest {
  tiles: Record<
    string,
    {
      nodeCount: number;
      edgeCount: number;
      bounds: { south: number; north: number; west: number; east: number };
    }
  >;
  totalNodes: number;
  totalEdges: number;
}
