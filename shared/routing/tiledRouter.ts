/**
 * tiledRouter.ts
 *
 * Main-thread tile manager for the tiled pathfinding graph.
 * Spawns a Web Worker and manages tile loading/caching.
 *
 * Refactored from execution-demo for shared use across apps.
 * Does NOT import OSDK directly — accepts a generic FoundryFileClient.
 *
 * Usage:
 *   const router = new TiledRouter({ datasetRid, client });
 *   await router.init();
 *   const route = await router.fetchRoute(lat1, lon1, lat2, lon2, 'tracked');
 *   const iso   = await router.fetchIsochrone(lat, lon, 'wheeled', 3600);
 *   router.dispose();
 */

import type {
  MobilityClass,
  OSRMRoute,
  IsochroneResult,
  ReachableNode,
  TileManifest,
  GeoJsonFeature,
} from './types';
import { TERRAIN_SPEEDS } from './types';

// ── Public client abstraction ─────────────────────────────────────────────────

/**
 * Generic file-fetching interface so callers can supply any implementation
 * (e.g. wrapping OSDK's `Files.content`) without coupling this module to OSDK.
 */
export interface FoundryFileClient {
  getFileContent(datasetRid: string, filePath: string): Promise<ArrayBuffer>;
}

// ── Internal extended manifest ────────────────────────────────────────────────

/**
 * The on-disk `manifest.json` carries grid metadata beyond the base
 * `TileManifest` type. We parse the full shape locally.
 */
interface FullManifest extends TileManifest {
  tileDeg: number;
  gridRows: number;
  gridCols: number;
  gridOrigin: { south: number; west: number };
  latScale: number;
  lonScale: number;
  distScale: number;
}

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_MAX_CACHED_TILES = 120;
const DEFAULT_TILE_BUFFER = 1;
const KM_PER_DEG_LAT = 111.32;
const DEFAULT_CONTOUR_INTERVALS = [3600, 7200, 14400, 28800];

/**
 * Maximum radius (km) for isochrone tile loading. Kept small for a fast
 * planning-grade tool — shows approximate reachability, not exact routing.
 * The Dijkstra in the worker stops at the time budget regardless.
 */
const MAX_ISOCHRONE_RADIUS_KM = 20;

/**
 * Maximum time budget (seconds) for isochrone computation. Even if the
 * phase is longer, a single order's movement window is capped here.
 * 8 hours = 28800 seconds — a realistic max for one movement order.
 */
const MAX_ISOCHRONE_TIME_SEC = 28800;

/**
 * Max concurrent tile fetches. Each tile = 2 HTTP requests (nodes + edges).
 * Browsers typically allow ~6 connections per origin, so 3 tiles in flight
 * keeps the pipeline full without exhausting the connection pool.
 */
const MAX_CONCURRENT_TILE_FETCHES = 3;

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Return the highest terrain speed (km/h) for a given mobility class. */
function maxSpeedForClass(mc: MobilityClass): number {
  const speeds = TERRAIN_SPEEDS[mc];
  return Math.max(...Object.values(speeds));
}

/** Wrap a coordinate ring into a GeoJSON Feature<Polygon>. */
function coordsToFeature(
  coords: [number, number][],
  properties: Record<string, unknown> = {},
): GeoJsonFeature {
  return {
    type: 'Feature',
    properties,
    geometry: { type: 'Polygon', coordinates: [coords] },
  };
}

// ── TiledRouter ───────────────────────────────────────────────────────────────

export class TiledRouter {
  private readonly datasetRid: string;
  private readonly client: FoundryFileClient;
  private readonly maxCachedTiles: number;
  private readonly tileBuffer: number;

  private manifest: FullManifest | null = null;
  private worker: Worker | null = null;
  private ready = false;
  private nextRequestId = 0;

  /** Tile keys already loaded into the worker. */
  private readonly loadedTileKeys = new Set<string>();

  /** LRU list – most recently used at end. */
  private readonly tileUsageOrder: string[] = [];

  /** Pending route / isochrone requests keyed by numeric ID. */
  private readonly pendingRequests = new Map<
    number,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    { resolve: (v: any) => void; reject: (e: Error) => void }
  >();

  /** Callbacks waiting for the worker to confirm a tile is loaded. */
  private readonly pendingTileLoads = new Map<string, () => void>();

  /** Resolved once the worker signals 'ready' during init(). */
  private initResolve: (() => void) | null = null;

  constructor(config: {
    datasetRid: string;
    client: FoundryFileClient;
    maxCachedTiles?: number;
    tileBuffer?: number;
  }) {
    this.datasetRid = config.datasetRid;
    this.client = config.client;
    this.maxCachedTiles = config.maxCachedTiles ?? DEFAULT_MAX_CACHED_TILES;
    this.tileBuffer = config.tileBuffer ?? DEFAULT_TILE_BUFFER;
  }

  // ── Lifecycle ─────────────────────────────────────────────────────────────

  /**
   * Fetch the manifest, spawn the Web Worker, and wait until the worker
   * reports ready.
   */
  async init(): Promise<void> {
    // 1. Fetch & parse manifest
    const manifestBuf = await this.client.getFileContent(
      this.datasetRid,
      'manifest.json',
    );
    const manifestText = new TextDecoder().decode(manifestBuf);
    this.manifest = JSON.parse(manifestText) as FullManifest;

    console.log(
      `[TiledRouter] manifest loaded – ` +
        `${Object.keys(this.manifest.tiles).length} tiles, ` +
        `${this.manifest.gridRows}×${this.manifest.gridCols} grid, ` +
        `${this.manifest.tileDeg}° per tile`,
    );

    // 2. Spawn worker
    this.worker = new Worker(
      new URL('./tiledRouterWorker.ts', import.meta.url),
      { type: 'module' },
    );

    // 3. Wire up message handler
    this.worker.onmessage = (e: MessageEvent) =>
      this.handleWorkerMessage(e.data);

    // 4. Send init & await 'ready'
    await new Promise<void>((resolve) => {
      this.initResolve = resolve;
      this.worker!.postMessage({ type: 'init' });
    });

    console.log('[TiledRouter] worker ready');
  }

  /** Terminate the worker and release all state. */
  dispose(): void {
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
    this.ready = false;
    this.manifest = null;
    this.loadedTileKeys.clear();
    this.tileUsageOrder.length = 0;
    this.pendingRequests.clear();
    this.pendingTileLoads.clear();
  }

  // ── Public API ────────────────────────────────────────────────────────────

  /**
   * Compute the shortest path between two WGS-84 points on the terrain graph.
   * Automatically fetches any tiles needed along the corridor.
   */
  async fetchRoute(
    fromLat: number,
    fromLon: number,
    toLat: number,
    toLon: number,
    mobilityClass: MobilityClass = 'tracked',
  ): Promise<OSRMRoute | null> {
    this.assertReady();

    // 1. Bounding box of the two endpoints + buffer ring
    const bufDeg = this.manifest!.tileDeg * this.tileBuffer;
    const neededKeys = this.getTileKeysForBbox(
      Math.min(fromLat, toLat) - bufDeg,
      Math.max(fromLat, toLat) + bufDeg,
      Math.min(fromLon, toLon) - bufDeg,
      Math.max(fromLon, toLon) + bufDeg,
    );

    // 2. Ensure tiles are loaded
    await this.ensureTilesLoaded(neededKeys);

    // 3. Send route request to worker
    const id = ++this.nextRequestId;
    return new Promise<OSRMRoute | null>((resolve, reject) => {
      this.pendingRequests.set(id, { resolve, reject });
      this.worker!.postMessage({
        type: 'route',
        id: String(id),
        fromLat,
        fromLon,
        toLat,
        toLon,
        mobilityClass,
      });
    });
  }

  /**
   * Compute reachability from a single point, returning an envelope polygon,
   * contour polygons at the requested intervals, and all reachable nodes.
   */
  async fetchIsochrone(
    fromLat: number,
    fromLon: number,
    mobilityClass: MobilityClass,
    timeBudgetSec: number,
    contourIntervalsSec?: number[],
  ): Promise<IsochroneResult | null> {
    this.assertReady();

    const intervals = contourIntervalsSec ?? DEFAULT_CONTOUR_INTERVALS;

    // Cap the time budget to a realistic single-order window
    const cappedTimeBudget = Math.min(timeBudgetSec, MAX_ISOCHRONE_TIME_SEC);

    // 1. Estimate the maximum reachable bounding box.
    //    Cap to MAX_ISOCHRONE_RADIUS_KM to avoid fetching hundreds of tiles.
    //    The worker's Dijkstra naturally respects the real time budget.
    const vMax = maxSpeedForClass(mobilityClass);
    const theoreticalKm = (vMax * timeBudgetSec) / 3600;
    const maxDistKm = Math.min(theoreticalKm, MAX_ISOCHRONE_RADIUS_KM);
    const latDelta = maxDistKm / KM_PER_DEG_LAT;
    const lonDelta =
      maxDistKm / (KM_PER_DEG_LAT * Math.cos((fromLat * Math.PI) / 180));

    // 2. Determine ALL tiles within that bbox
    const tileKeys = this.getTileKeysForBbox(
      fromLat - latDelta,
      fromLat + latDelta,
      fromLon - lonDelta,
      fromLon + lonDelta,
    );

    // 3. Load them
    await this.ensureTilesLoaded(tileKeys);

    // 4. Send isochrone request to worker
    const id = ++this.nextRequestId;

    return new Promise<IsochroneResult | null>((resolve, reject) => {
      this.pendingRequests.set(id, {
        resolve: (raw: {
          nodes: ReachableNode[];
          envelopeCoords: [number, number][] | null;
          contourCoords: [number, number][][];
        } | null) => {
          if (!raw) {
            resolve(null);
            return;
          }

          // 5. Reconstruct full IsochroneResult with GeoJSON features
          const envelope: GeoJsonFeature | null = raw.envelopeCoords
            ? coordsToFeature(raw.envelopeCoords, { type: 'envelope' })
            : null;

          const contours: GeoJsonFeature[] = raw.contourCoords.map(
            (ring, i) =>
              coordsToFeature(ring, {
                type: 'contour',
                intervalSec: intervals[i] ?? 0,
              }),
          );

          resolve({
            reachableNodes: raw.nodes,
            envelope,
            impassableWithinEnvelope: null,
            contours,
            sourceLat: fromLat,
            sourceLon: fromLon,
            timeBudgetSec,
            mobilityClass,
          });
        },
        reject,
      });

      this.worker!.postMessage({
        type: 'isochrone',
        id: String(id),
        fromLat,
        fromLon,
        mobilityClass,
        timeBudgetSec: cappedTimeBudget,
        contourIntervalsSec: intervals.filter(i => i <= cappedTimeBudget),
      });
    });
  }

  /**
   * Pre-load tiles around a set of positions (e.g. initial unit locations)
   * so that subsequent route/isochrone requests are fast.
   */
  async preloadTilesAround(positions: [number, number][]): Promise<void> {
    if (!this.manifest || positions.length === 0) return;

    const bufDeg = this.manifest.tileDeg * this.tileBuffer;
    let minLat = Infinity;
    let maxLat = -Infinity;
    let minLon = Infinity;
    let maxLon = -Infinity;

    for (const [lat, lon] of positions) {
      if (lat < minLat) minLat = lat;
      if (lat > maxLat) maxLat = lat;
      if (lon < minLon) minLon = lon;
      if (lon > maxLon) maxLon = lon;
    }

    const keys = this.getTileKeysForBbox(
      minLat - bufDeg,
      maxLat + bufDeg,
      minLon - bufDeg,
      maxLon + bufDeg,
    );

    if (keys.length > 0) {
      console.log(
        `[TiledRouter] pre-loading ${keys.length} tiles around ${positions.length} positions`,
      );
      await this.ensureTilesLoaded(keys);
    }
  }

  // ── Private: tile management ──────────────────────────────────────────────

  /**
   * Ensure every tile key in the list is loaded into the worker.
   * Fetches missing tiles from the Foundry dataset and transfers the binary
   * data to the worker via `addTile` messages with Transferable buffers.
   */
  private async ensureTilesLoaded(tileKeys: string[]): Promise<void> {
    // Filter to tiles not yet loaded
    const toFetch = tileKeys.filter((k) => !this.loadedTileKeys.has(k));
    if (toFetch.length === 0) {
      // Still update LRU for already-loaded tiles
      this.touchLru(tileKeys);
      return;
    }

    console.log(
      `[TiledRouter] fetching ${toFetch.length} tiles (max ${MAX_CONCURRENT_TILE_FETCHES} concurrent)`,
    );

    // Filter to tiles that actually exist in the manifest
    const validKeys = toFetch.filter((k) => !!this.manifest!.tiles[k]);

    // Fetch tiles with concurrency limiting to avoid ERR_INSUFFICIENT_RESOURCES.
    // Each tile requires 2 HTTP requests (nodes + edges), so we limit to
    // MAX_CONCURRENT_TILE_FETCHES tiles in flight at once.
    const validResults: {
      key: string;
      nodesBuffer: ArrayBuffer;
      edgesBuffer: ArrayBuffer;
    }[] = [];

    for (let i = 0; i < validKeys.length; i += MAX_CONCURRENT_TILE_FETCHES) {
      const batch = validKeys.slice(i, i + MAX_CONCURRENT_TILE_FETCHES);
      const batchResults = await Promise.all(
        batch.map(async (key) => {
          try {
            const [nodesBuffer, edgesBuffer] = await Promise.all([
              this.client.getFileContent(this.datasetRid, `tile_${key}_nodes.bin`),
              this.client.getFileContent(this.datasetRid, `tile_${key}_edges.bin`),
            ]);
            return { key, nodesBuffer, edgesBuffer };
          } catch (err) {
            console.warn(`[TiledRouter] failed to fetch tile ${key}:`, err);
            return null;
          }
        }),
      );
      for (const r of batchResults) {
        if (r) validResults.push(r);
      }
    }

    if (validResults.length === 0) {
      this.touchLru(tileKeys);
      return;
    }

    // Send each tile to the worker and await acknowledgement.
    // The worker replies with { type: 'tilesLoaded', keys: [...] } after
    // each addTile. We resolve per-key callbacks to know when all are done.
    await new Promise<void>((resolve) => {
      let remaining = validResults.length;
      const onTileAcknowledged = () => {
        if (--remaining === 0) resolve();
      };

      for (const r of validResults) {
        this.pendingTileLoads.set(r.key, onTileAcknowledged);

        // Transfer buffers (zero-copy) so ArrayBuffers are moved, not copied
        this.worker!.postMessage(
          {
            type: 'addTile',
            tileKey: r.key,
            nodesBuffer: r.nodesBuffer,
            edgesBuffer: r.edgesBuffer,
          },
          [r.nodesBuffer, r.edgesBuffer],
        );
      }
    });

    // Mark loaded
    for (const r of validResults) {
      this.loadedTileKeys.add(r.key);
    }

    // Update LRU for all requested keys (loaded + already-cached)
    this.touchLru(tileKeys);

    // Evict oldest tiles if over the cache limit
    while (this.tileUsageOrder.length > this.maxCachedTiles) {
      const evicted = this.tileUsageOrder.shift()!;
      this.loadedTileKeys.delete(evicted);
      // NOTE: we don't currently send an unload message to the worker.
      // For very long-running sessions consider adding { type: 'removeTile' }.
    }
  }

  /** Move the given keys to the end of the LRU list. */
  private touchLru(keys: string[]): void {
    for (const k of keys) {
      const idx = this.tileUsageOrder.indexOf(k);
      if (idx >= 0) this.tileUsageOrder.splice(idx, 1);
      this.tileUsageOrder.push(k);
    }
  }

  /**
   * Return all tile keys from the manifest whose grid cell overlaps the
   * given lat/lon bounding box. Tile keys are `{row}_{col}` strings.
   */
  private getTileKeysForBbox(
    minLat: number,
    maxLat: number,
    minLon: number,
    maxLon: number,
  ): string[] {
    if (!this.manifest) return [];
    const { tileDeg, gridOrigin, gridRows, gridCols, tiles } = this.manifest;

    const rMin = Math.max(
      0,
      Math.floor((minLat - gridOrigin.south) / tileDeg),
    );
    const rMax = Math.min(
      gridRows - 1,
      Math.floor((maxLat - gridOrigin.south) / tileDeg),
    );
    const cMin = Math.max(
      0,
      Math.floor((minLon - gridOrigin.west) / tileDeg),
    );
    const cMax = Math.min(
      gridCols - 1,
      Math.floor((maxLon - gridOrigin.west) / tileDeg),
    );

    const keys: string[] = [];
    for (let r = rMin; r <= rMax; r++) {
      for (let c = cMin; c <= cMax; c++) {
        const k = `${r}_${c}`;
        if (tiles[k]) keys.push(k);
      }
    }
    return keys;
  }

  // ── Private: worker message handling ──────────────────────────────────────

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private handleWorkerMessage(msg: any): void {
    switch (msg.type) {
      // ── init acknowledgement ──────────────────────────────────────────
      case 'ready': {
        this.ready = true;
        if (this.initResolve) {
          this.initResolve();
          this.initResolve = null;
        }
        break;
      }

      // ── tile-load acknowledgement ─────────────────────────────────────
      case 'tilesLoaded': {
        const keys = msg.keys as string[];
        for (const key of keys) {
          const cb = this.pendingTileLoads.get(key);
          if (cb) {
            this.pendingTileLoads.delete(key);
            cb();
          }
        }
        break;
      }

      // ── route result ──────────────────────────────────────────────────
      case 'routeResult': {
        const id = Number(msg.id);
        const pending = this.pendingRequests.get(id);
        if (pending) {
          this.pendingRequests.delete(id);
          pending.resolve(msg.route as OSRMRoute | null);
        }
        break;
      }

      // ── isochrone result ──────────────────────────────────────────────
      case 'isochroneResult': {
        const id = Number(msg.id);
        const pending = this.pendingRequests.get(id);
        if (pending) {
          this.pendingRequests.delete(id);
          pending.resolve({
            nodes: msg.nodes as ReachableNode[],
            envelopeCoords: msg.envelopeCoords as
              | [number, number][]
              | null,
            contourCoords: msg.contourCoords as [number, number][][],
          });
        }
        break;
      }

      default:
        break;
    }
  }

  /** Throw if the router has not been initialised yet. */
  private assertReady(): void {
    if (!this.manifest || !this.worker || !this.ready) {
      throw new Error('[TiledRouter] not initialised – call init() first');
    }
  }
}
