/**
 * terrainRaster.ts
 *
 * Loads a compact terrain cost raster (terrain_grid.bin) and provides
 * near-instant BFS flood-fill isochrone computation (<50ms).
 *
 * This replaces the slow tile-by-tile graph approach for the planning-grade
 * reachability envelope. The raster is a single file (~100-300 KB) loaded once.
 *
 * Binary format:
 *   Header (32 bytes): float64 minLat, float64 minLon, float64 cellSizeDeg,
 *                      uint32 numRows, uint32 numCols
 *   Body: Uint8Array[numRows × numCols] - terrain flags per cell
 *         0=open, 1=road, 2=forest, 3=marsh
 */

import type { MobilityClass, IsochroneResult, ReachableNode, OSRMRoute } from './types';
import { TERRAIN_SPEEDS } from './types';
import type { GeoJsonFeature } from './types';

// ── Min-heap for A* ───────────────────────────────────────────────────────────

class MinHeap {
  private heap: [number, number][] = []; // [priority, index]
  push(priority: number, index: number) {
    this.heap.push([priority, index]);
    let i = this.heap.length - 1;
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (this.heap[p][0] <= this.heap[i][0]) break;
      [this.heap[p], this.heap[i]] = [this.heap[i], this.heap[p]];
      i = p;
    }
  }
  pop(): [number, number] | undefined {
    if (this.heap.length === 0) return undefined;
    const top = this.heap[0];
    const last = this.heap.pop()!;
    if (this.heap.length > 0) {
      this.heap[0] = last;
      let i = 0;
      const n = this.heap.length;
      while (true) {
        let s = i;
        const l = 2 * i + 1, r = 2 * i + 2;
        if (l < n && this.heap[l][0] < this.heap[s][0]) s = l;
        if (r < n && this.heap[r][0] < this.heap[s][0]) s = r;
        if (s === i) break;
        [this.heap[s], this.heap[i]] = [this.heap[i], this.heap[s]];
        i = s;
      }
    }
    return top;
  }
  get size() { return this.heap.length; }
}
import type { FoundryFileClient } from './tiledRouter';
import { DEFAULT_TERRAIN } from './terrainDatasets';

/**
 * Eight-connected movement: orthogonals first, then the four diagonals.
 *
 * ⚠ THE ORDER MATTERS — indices 0-3 are orthogonal and 4-7 are diagonal, and
 * everything below keys off DIAGONAL_FROM to know which cost to charge.
 */
const DIR_R = [-1, 1, 0, 0, -1, -1, 1, 1];
const DIR_C = [0, 0, -1, 1, -1, 1, -1, 1];
const DIAGONAL_FROM = 4;

/**
 * A diagonal step crosses sqrt(2) cells of ground, so it costs that much more.
 *
 * ⚠ OMITTING THIS DOES NOT LOOK LIKE A BUG, WHICH IS WHY IT IS NAMED. Charging
 * a diagonal the same as an orthogonal makes it cheaper PER METRE, so the
 * router starts zig-zagging on purpose to exploit it — and the resulting paths
 * look SMOOTHER than the staircases they replaced, so the error reads as an
 * improvement.
 */
const DIAGONAL_COST = Math.SQRT2;

/**
 * May a diagonal step be taken, or would it cut a corner?
 *
 * A diagonal between two impassable orthogonal neighbours slips through the
 * corner of a lake or a building — geometrically through ground the vehicle
 * cannot occupy. Both orthogonals must be passable, which is the strict rule
 * and the right one for vehicles.
 */
function cornerOpen(
  cellCosts: Float32Array,
  rows: number,
  cols: number,
  r: number,
  c: number,
  d: number,
): boolean {
  const vr = r + DIR_R[d];
  const hc = c + DIR_C[d];
  const vertical = vr >= 0 && vr < rows ? cellCosts[vr * cols + c] : -1;
  const horizontal = hc >= 0 && hc < cols ? cellCosts[r * cols + hc] : -1;
  return vertical >= 0 && horizontal >= 0;
}


// ── Config ────────────────────────────────────────────────────────────────────

// The raster to load is chosen per scenario — see terrainDatasets.ts. It used
// to be a constant pointing at the Narva build, which quietly mis-costed every
// scenario fought anywhere else.
const RASTER_FILE = 'terrain_grid.bin';

// ── Calibration offset ────────────────────────────────────────────────────────
// Corrects the terrain raster to align with the base map.
// Use the TERRAIN debug toggle to visually compare, then adjust these.
// Positive LAT_OFFSET = shift envelope south. Positive LON_OFFSET = shift east.
const LAT_OFFSET = 0;
const LON_OFFSET = 0;

// ── Types ─────────────────────────────────────────────────────────────────────

interface RasterGrid {
  minLat: number;
  minLon: number;
  cellSize: number;
  numRows: number;
  numCols: number;
  data: Uint8Array;
}

/** A loaded raster's grid as plain, transferable data (see `TerrainRaster.exportGrid`). */
export type RasterGridData = Readonly<RasterGrid>;

// ── Public API ────────────────────────────────────────────────────────────────

export class TerrainRaster {
  private grid: RasterGrid | null = null;
  private loadPromise: Promise<void> | null = null;

  constructor(
    private client: FoundryFileClient,
    /** Dataset holding terrain_grid.bin. Defaults to the default scenario's. */
    private datasetRid: string = DEFAULT_TERRAIN.rasterRid,
  ) {}

  /** Load the terrain raster. Call once — subsequent calls return cached data. */
  async load(): Promise<void> {
    if (this.grid) return;
    if (this.loadPromise) return this.loadPromise;

    this.loadPromise = this._doLoad();
    await this.loadPromise;
  }

  get isLoaded(): boolean {
    return this.grid !== null;
  }

  /**
   * The loaded grid, after its water clean-up, as plain data: five numbers
   * and one byte array. Everything this raster answers comes from it, so a
   * Web Worker can rebuild the same raster with `fromGrid` — transfer
   * `data.buffer` — instead of fetching and cleaning the file again.
   * Null before it loads.
   */
  exportGrid(): RasterGridData | null {
    if (!this.grid) return null;
    const { minLat, minLon, cellSize, numRows, numCols, data } = this.grid;
    return { minLat, minLon, cellSize, numRows, numCols, data: new Uint8Array(data) };
  }

  /** A loaded raster from `exportGrid`'s data. It cannot fetch: it already has its grid. */
  static fromGrid(grid: RasterGridData, datasetRid?: string): TerrainRaster {
    if (grid.data.length !== grid.numRows * grid.numCols) {
      throw new Error(`[TerrainRaster] grid size mismatch: expected ${grid.numRows * grid.numCols}, got ${grid.data.length}`);
    }
    const noFetch: FoundryFileClient = {
      getFileContent: () => Promise.reject(new Error("[TerrainRaster] built from a grid; nothing to fetch")),
    };
    const raster = new TerrainRaster(noFetch, datasetRid);
    raster.grid = { ...grid };
    return raster;
  }

  /** Exposes calibration offsets so external debug overlays can stay in sync. */
  get latOffset(): number { return LAT_OFFSET; }
  get lonOffset(): number { return LON_OFFSET; }

  private async _doLoad(): Promise<void> {
    console.log(`[TerrainRaster] loading ${RASTER_FILE} from ${this.datasetRid} ...`);
    const buf = await this.client.getFileContent(this.datasetRid, RASTER_FILE);
    const view = new DataView(buf);

    // Parse header (32 bytes)
    const minLat = view.getFloat64(0, true);
    const minLon = view.getFloat64(8, true);
    const cellSize = view.getFloat64(16, true);
    const numRows = view.getUint32(24, true);
    const numCols = view.getUint32(28, true);

    // Body
    const data = new Uint8Array(buf, 32);
    if (data.length !== numRows * numCols) {
      throw new Error(
        `[TerrainRaster] data size mismatch: expected ${numRows * numCols}, got ${data.length}`
      );
    }

    const writable = new Uint8Array(data); // data is already a Uint8Array view

    // ── Step 1: Flood-fill connected water bodies, tag each with a size ──
    // We want to keep large water bodies (rivers, lakes) as impassable but
    // remove isolated single-cell or tiny water specks (raster noise / tile
    // gaps) that would otherwise create spurious narrow barriers.
    //
    // Algorithm:
    //   1. Label each connected component of 255 cells via BFS.
    //   2. Components with area < MIN_WATER_CELLS are noise — convert to 0 (open).
    //   3. Remaining large bodies stay as 255.
    //
    // MIN_WATER_CELLS ≈ 3 cells at ~220 m resolution = ~660 m minimum river width.
    // This keeps the Narva River and Emajõgi (wide rivers) impassable while
    // removing single-pixel tile-join artifacts.
    const MIN_WATER_CELLS = 3;

    const labels = new Int32Array(numRows * numCols).fill(-1); // -1 = unlabelled
    const componentSize: number[] = [];
    let nextLabel = 0;

    for (let r = 0; r < numRows; r++) {
      for (let c = 0; c < numCols; c++) {
        const idx = r * numCols + c;
        if (writable[idx] !== 255 || labels[idx] !== -1) continue;

        // BFS to find all connected 255 cells
        const label = nextLabel++;
        componentSize.push(0);
        const queue: number[] = [idx];
        labels[idx] = label;

        let qi = 0;
        while (qi < queue.length) {
          const cur = queue[qi++];
          componentSize[label]++;
          const cr = (cur / numCols) | 0;
          const cc = cur % numCols;
          for (const [dr, dc] of [[-1,0],[1,0],[0,-1],[0,1]] as [number,number][]) {
            const nr = cr + dr;
            const nc = cc + dc;
            if (nr < 0 || nr >= numRows || nc < 0 || nc >= numCols) continue;
            const nIdx = nr * numCols + nc;
            if (writable[nIdx] === 255 && labels[nIdx] === -1) {
              labels[nIdx] = label;
              queue.push(nIdx);
            }
          }
        }
      }
    }

    // Remove tiny isolated water specs (noise)
    let noiseRemoved = 0;
    for (let i = 0; i < numRows * numCols; i++) {
      const lbl = labels[i];
      if (lbl !== -1 && componentSize[lbl] < MIN_WATER_CELLS) {
        writable[i] = 0; // treat as open ground
        noiseRemoved++;
      }
    }
    if (noiseRemoved > 0) console.log(`[TerrainRaster] removed ${noiseRemoved} noise water cells (components < ${MIN_WATER_CELLS} cells)`);

    // ── Step 2: Dilate remaining water by 1 cell to close narrow gaps ───
    // Water bodies in the raster are often 1 cell wide at tile boundaries.
    // A single-cell-wide river is easily bypassed diagonally. Expanding each
    // water cell by 1 cell in 4-connected directions makes barriers solid.
    //
    // We write into a separate buffer to avoid cascading the dilation.
    const dilated = new Uint8Array(writable);
    let dilateCount = 0;
    for (let r = 0; r < numRows; r++) {
      for (let c = 0; c < numCols; c++) {
        if (writable[r * numCols + c] !== 255) continue;
        // Expand into 4-connected non-water neighbours
        for (const [dr, dc] of [[-1,0],[1,0],[0,-1],[0,1]] as [number,number][]) {
          const nr = r + dr;
          const nc = c + dc;
          if (nr < 0 || nr >= numRows || nc < 0 || nc >= numCols) continue;
          const nIdx = nr * numCols + nc;
          if (dilated[nIdx] !== 255) {
            dilated[nIdx] = 255;
            dilateCount++;
          }
        }
      }
    }
    // Copy dilation back
    for (let i = 0; i < dilated.length; i++) writable[i] = dilated[i];
    if (dilateCount > 0) console.log(`[TerrainRaster] water dilation: ${dilateCount} cells added to close gaps`);

    this.grid = { minLat, minLon, cellSize, numRows, numCols, data: writable };
    const sizeKB = (buf.byteLength / 1024).toFixed(1);
    console.log(
      `[TerrainRaster] loaded: ${numRows}×${numCols} grid, ` +
      `${sizeKB} KB, cell=${(cellSize * 111320).toFixed(0)}m, ` +
      `bounds=[${minLat.toFixed(3)}, ${minLon.toFixed(3)}] to ` +
      `[${(minLat + numRows * cellSize).toFixed(3)}, ${(minLon + numCols * cellSize).toFixed(3)}]`
    );
  }

  /**
   * Compute a fast reachability isochrone via BFS flood-fill on the raster grid.
   * Returns an IsochroneResult with the reachability envelope.
   *
   * @param lat - Unit latitude
   * @param lon - Unit longitude
   * @param mobilityClass - Unit mobility type
   * @param timeBudgetSec - Maximum travel time in seconds
   * @param contourIntervalsSec - Time intervals for contour rings
   */
  computeIsochrone(
    lat: number,
    lon: number,
    mobilityClass: MobilityClass,
    timeBudgetSec: number,
    contourIntervalsSec: number[] = [3600, 7200, 14400],
  ): IsochroneResult | null {
    if (!this.grid) return null;

    const { minLat, minLon, cellSize, numRows, numCols, data } = this.grid;
    const speeds = TERRAIN_SPEEDS[mobilityClass];

    // Convert lat/lon to grid cell (with calibration offset).
    // Grid is stored top-down (row 0 = north) but indexed bottom-up,
    // so flip the row: row 0 in file = north = numRows-1 logically.
    const adjLat = lat + LAT_OFFSET;
    const adjLon = lon + LON_OFFSET;
    const startRow = numRows - 1 - Math.floor((adjLat - minLat) / cellSize);
    const startCol = Math.floor((adjLon - minLon) / cellSize);

    if (startRow < 0 || startRow >= numRows || startCol < 0 || startCol >= numCols) {
      console.warn('[TerrainRaster] unit position outside raster bounds');
      return null;
    }

    // Work on a LOCAL WINDOW around the unit to avoid allocating arrays
    // for the entire grid (which can be 15M+ cells). The window is sized
    // to the max possible reach: fastest speed × time budget.
    const maxSpeedKmh = Math.max(...Object.values(speeds).filter(s => s > 0));
    if (maxSpeedKmh === 0) return null;
    const maxReachKm = (maxSpeedKmh * timeBudgetSec) / 3600;
    const cellDistKm = cellSize * 111.32;
    // Cap window to 150 cells radius (~33km at 220m/cell) to keep memory bounded
    const maxReachCells = Math.min(Math.ceil(maxReachKm / cellDistKm) + 5, 150);

    // Clamp window to grid bounds
    const winRowMin = Math.max(0, startRow - maxReachCells);
    const winRowMax = Math.min(numRows - 1, startRow + maxReachCells);
    const winColMin = Math.max(0, startCol - maxReachCells);
    const winColMax = Math.min(numCols - 1, startCol + maxReachCells);
    const winRows = winRowMax - winRowMin + 1;
    const winCols = winColMax - winColMin + 1;
    const winSize = winRows * winCols;

    // Local start position within the window
    const localStartRow = startRow - winRowMin;
    const localStartCol = startCol - winColMin;

    console.log(`[TerrainRaster] BFS window: ${winRows}×${winCols} = ${winSize} cells (max reach ${maxReachCells} cells / ${maxReachKm.toFixed(0)} km)`);

    // Iterative relaxation on the local window (Bellman-Ford style).
    // Dead simple: sweep the grid repeatedly, updating costs from neighbours.
    // Converges in ~5-15 passes. No queues, no overflow, proper cost handling.
    // Pre-compute per-cell traversal cost (seconds to enter that cell)
    const cellCosts = new Float32Array(winSize).fill(-1); // -1 = impassable
    for (let lr = 0; lr < winRows; lr++) {
      for (let lc = 0; lc < winCols; lc++) {
        const gr = winRowMin + lr;
        const gc = winColMin + lc;
        const globalIdx = gr * numCols + gc;
        const terrain = data[globalIdx];
        if (terrain === 255) continue; // deep water — impassable
        const speed = speeds[terrain] ?? 0;
        if (speed <= 0) continue; // impassable for this mobility class
        cellCosts[lr * winCols + lc] = (cellDistKm / speed) * 3600;
      }
    }

    const costs = new Float32Array(winSize).fill(Infinity);
    const startIdx = localStartRow * winCols + localStartCol;
    costs[startIdx] = 0;

    // Sweep forward and backward alternately for faster convergence
    const MAX_PASSES = 20;
    for (let pass = 0; pass < MAX_PASSES; pass++) {
      let changed = 0;

      // Forward sweep (top-left to bottom-right)
      for (let lr = 0; lr < winRows; lr++) {
        for (let lc = 0; lc < winCols; lc++) {
          const idx = lr * winCols + lc;
          const myCost = costs[idx];
          if (myCost >= timeBudgetSec) continue;

          for (let d = 0; d < DIR_R.length; d++) {
            const nlr = lr + DIR_R[d];
            const nlc = lc + DIR_C[d];
            if (nlr < 0 || nlr >= winRows || nlc < 0 || nlc >= winCols) continue;
            if (d >= DIAGONAL_FROM && !cornerOpen(cellCosts, winRows, winCols, lr, lc, d)) continue;
            const nIdx = nlr * winCols + nlc;
            const cc = cellCosts[nIdx];
            if (cc < 0) continue; // impassable
            const newCost = myCost + (d >= DIAGONAL_FROM ? cc * DIAGONAL_COST : cc);
            if (newCost < costs[nIdx] && newCost <= timeBudgetSec) {
              costs[nIdx] = newCost;
              changed++;
            }
          }
        }
      }

      // Backward sweep (bottom-right to top-left)
      for (let lr = winRows - 1; lr >= 0; lr--) {
        for (let lc = winCols - 1; lc >= 0; lc--) {
          const idx = lr * winCols + lc;
          const myCost = costs[idx];
          if (myCost >= timeBudgetSec) continue;

          for (let d = 0; d < DIR_R.length; d++) {
            const nlr = lr + DIR_R[d];
            const nlc = lc + DIR_C[d];
            if (nlr < 0 || nlr >= winRows || nlc < 0 || nlc >= winCols) continue;
            if (d >= DIAGONAL_FROM && !cornerOpen(cellCosts, winRows, winCols, lr, lc, d)) continue;
            const nIdx = nlr * winCols + nlc;
            const cc = cellCosts[nIdx];
            if (cc < 0) continue;
            const newCost = myCost + (d >= DIAGONAL_FROM ? cc * DIAGONAL_COST : cc);
            if (newCost < costs[nIdx] && newCost <= timeBudgetSec) {
              costs[nIdx] = newCost;
              changed++;
            }
          }
        }
      }

      if (changed === 0) {
        console.log(`[TerrainRaster] converged after ${pass + 1} passes`);
        break;
      }
    }

    // Collect reachable cells (map local indices back to global lat/lon).
    // Output coords are raw raster WGS84 — no offset subtraction. The offset
    // only shifts which raster cell we start from; it does not relocate output.
    const reachableNodes: ReachableNode[] = [];
    for (let i = 0; i < winSize; i++) {
      if (costs[i] <= timeBudgetSec) {
        const localRow = (i / winCols) | 0;
        const localCol = i % winCols;
        const globalRow = winRowMin + localRow;
        const globalCol = winColMin + localCol;
        reachableNodes.push({
          lat: minLat + (numRows - 1 - globalRow + 0.5) * cellSize,
          lon: minLon + (globalCol + 0.5) * cellSize,
          costSec: costs[i],
          globalNodeId: globalRow * numCols + globalCol,
        });
      }
    }

    // Build MultiPolygon of impassable cells that border reachable cells.
    // Each impassable cell (water/blocked terrain) becomes a small rectangle.
    // Only cells adjacent to at least one reachable cell are included — this
    // avoids rendering water far outside the envelope while capturing ponds,
    // rivers, and lakes that sit within or on the edge of the reachable zone.
    const impassablePolys: [number, number][][][] = [];
    for (let lr = 0; lr < winRows; lr++) {
      for (let lc = 0; lc < winCols; lc++) {
        const idx = lr * winCols + lc;
        if (cellCosts[idx] >= 0) continue; // traversable cell — skip
        if (costs[idx] <= timeBudgetSec) continue; // reachable — skip

        // Only include if at least one 4-connected neighbour is reachable
        let bordersReachable = false;
        for (const [dr, dc] of [[-1,0],[1,0],[0,-1],[0,1]] as [number,number][]) {
          const nr = lr + dr;
          const nc = lc + dc;
          if (nr >= 0 && nr < winRows && nc >= 0 && nc < winCols) {
            if (costs[nr * winCols + nc] <= timeBudgetSec) {
              bordersReachable = true;
              break;
            }
          }
        }
        if (!bordersReachable) continue;

        const globalRow = winRowMin + lr;
        const globalCol = winColMin + lc;
        const s = minLat + (numRows - 1 - globalRow) * cellSize;
        const w = minLon + globalCol * cellSize;
        const n = s + cellSize;
        const e = w + cellSize;
        impassablePolys.push([[[w,s],[e,s],[e,n],[w,n],[w,s]]]);
      }
    }

    console.log(`[TerrainRaster] isochrone: ${reachableNodes.length} reachable, ${impassablePolys.length} impassable border cells, ${mobilityClass}/${timeBudgetSec}s`);

    // The hull source must match where the nodes sit — which is the adjusted
    // raster position (unit coords shifted by LAT_OFFSET for cell lookup).
    const hullSourceLat = lat + LAT_OFFSET;
    const hullSourceLon = lon + LON_OFFSET;

    // Generate envelope using sector hull
    const envelope = this._sectorHull(hullSourceLat, hullSourceLon, reachableNodes, timeBudgetSec);

    // Generate contours
    const contours: GeoJsonFeature[] = [];
    for (const interval of contourIntervalsSec) {
      if (interval >= timeBudgetSec) continue;
      const filtered = reachableNodes.filter(n => n.costSec <= interval);
      if (filtered.length >= 3) {
        const hull = this._sectorHull(hullSourceLat, hullSourceLon, filtered, interval);
        if (hull) contours.push(hull);
      }
    }

    // Build impassable MultiPolygon (water/blocked cells within envelope)
    const impassableWithinEnvelope: GeoJsonFeature | null = impassablePolys.length > 0
      ? {
          type: 'Feature',
          properties: {},
          geometry: { type: 'MultiPolygon', coordinates: impassablePolys },
        }
      : null;

    return {
      reachableNodes,
      envelope,
      impassableWithinEnvelope,
      contours,
      sourceLat: lat,
      sourceLon: lon,
      timeBudgetSec,
      mobilityClass,
    };
  }

  // ── Terrain query ──────────────────────────────────────────────

  /**
   * The raw terrain flag at a position, or null outside coverage.
   *
   * NULL IS NOT "OPEN GROUND" AND THE DISTINCTION MATTERS. Off the edge of the
   * raster we know nothing; inside it, flag 0 means we know it is open. A
   * caller that conflates the two will quietly play a whole game on ground it
   * has no data for — which is how BGWS came to be wired to this: its default
   * board sits near Gdansk and the default raster covers Kaliningrad, so
   * "unknown" had to be answerable.
   *
   * Added for BGWS's terrain sampler; `isPassable` is now a reading of it.
   */
  flagAt(lat: number, lon: number): number | null {
    if (!this.grid) return null;
    const { minLat, minLon, cellSize, numRows, numCols, data } = this.grid;
    const row = numRows - 1 - Math.floor((lat + LAT_OFFSET - minLat) / cellSize);
    const col = Math.floor((lon + LON_OFFSET - minLon) / cellSize);
    if (row < 0 || row >= numRows || col < 0 || col >= numCols) return null;
    return data[row * numCols + col];
  }

  /**
   * The area this raster covers, or null before it loads.
   *
   * Exposed so a caller can put its board somewhere the data actually is,
   * rather than discovering the hard way that every cell came back unknown.
   */
  get bounds(): { minLat: number; minLon: number; maxLat: number; maxLon: number } | null {
    if (!this.grid) return null;
    const { minLat, minLon, cellSize, numRows, numCols } = this.grid;
    return {
      minLat: minLat - LAT_OFFSET,
      minLon: minLon - LON_OFFSET,
      maxLat: minLat - LAT_OFFSET + numRows * cellSize,
      maxLon: minLon - LON_OFFSET + numCols * cellSize,
    };
  }

  /**
   * Check whether a lat/lon position is on passable terrain.
   * Returns false for water (255), out-of-bounds, or unloaded grid.
   */
  isPassable(lat: number, lon: number): boolean {
    if (!this.grid) return true; // allow placement if raster isn't loaded
    const flag = this.flagAt(lat, lon);
    if (flag === null) return false; // outside coverage
    return flag !== 255; // 255 = water/impassable
  }

  // ── A* point-to-point routing on the terrain raster ─────────────

  /**
   * Compute a route from (fromLat, fromLon) to (toLat, toLon) using A*
   * on the terrain raster grid. Uses the same terrain speeds as the BFS
   * isochrone, ensuring consistency between reachability and routing.
   *
   * Returns an OSRMRoute compatible with the RoutePreview component,
   * or null if no path exists.
   */
  async fetchRoute(
    fromLat: number,
    fromLon: number,
    toLat: number,
    toLon: number,
    mobilityClass: MobilityClass = 'tracked',
  ): Promise<OSRMRoute | null> {
    return this.route(fromLat, fromLon, toLat, toLon, mobilityClass);
  }

  /**
   * The same A*, synchronously.
   *
   * ⚠ IT WAS NEVER ASYNCHRONOUS. `fetchRoute` awaits nothing: the grid is
   * already in memory and the search is a loop. The Promise was interface
   * decoration, and it cost a real caller — BGWS resolves a turn's options
   * synchronously, so an async router could not be consulted while deciding
   * where a unit may go, and the game had to make do with bearings.
   *
   * `fetchRoute` is kept as the async wrapper so no existing caller changes.
   */
  route(
    fromLat: number,
    fromLon: number,
    toLat: number,
    toLon: number,
    mobilityClass: MobilityClass = 'tracked',
  ): OSRMRoute | null {
    if (!this.grid) return null;

    const { minLat, minLon, cellSize, numRows, numCols, data } = this.grid;
    const speeds = TERRAIN_SPEEDS[mobilityClass];

    // Convert lat/lon to grid coordinates (row 0 = north)
    const toGridRow = (lat: number) =>
      numRows - 1 - Math.floor((lat + LAT_OFFSET - minLat) / cellSize);
    const toGridCol = (lon: number) =>
      Math.floor((lon + LON_OFFSET - minLon) / cellSize);

    const fromRow = toGridRow(fromLat);
    const fromCol = toGridCol(fromLon);
    const toRow = toGridRow(toLat);
    const toCol = toGridCol(toLon);

    // Bounds check
    if (fromRow < 0 || fromRow >= numRows || fromCol < 0 || fromCol >= numCols) return null;
    if (toRow < 0 || toRow >= numRows || toCol < 0 || toCol >= numCols) return null;

    // Create a window covering both endpoints with generous buffer
    // to allow routes that detour around obstacles.
    const straightDist = Math.sqrt((fromRow - toRow) ** 2 + (fromCol - toCol) ** 2);
    const buffer = Math.max(40, Math.ceil(straightDist * 0.6));
    const winRowMin = Math.max(0, Math.min(fromRow, toRow) - buffer);
    const winRowMax = Math.min(numRows - 1, Math.max(fromRow, toRow) + buffer);
    const winColMin = Math.max(0, Math.min(fromCol, toCol) - buffer);
    const winColMax = Math.min(numCols - 1, Math.max(fromCol, toCol) + buffer);
    const winRows = winRowMax - winRowMin + 1;
    const winCols = winColMax - winColMin + 1;

    // Local coordinates
    const lFromIdx = (fromRow - winRowMin) * winCols + (fromCol - winColMin);
    const lToIdx = (toRow - winRowMin) * winCols + (toCol - winColMin);

    const cellDistKm = cellSize * 111.32;

    // Pre-compute per-cell traversal cost (seconds to cross one cell)
    const cellCosts = new Float32Array(winRows * winCols).fill(-1);
    for (let lr = 0; lr < winRows; lr++) {
      for (let lc = 0; lc < winCols; lc++) {
        const terrain = data[(winRowMin + lr) * numCols + (winColMin + lc)];
        if (terrain === 255) continue;
        const speed = speeds[terrain] ?? 0;
        if (speed <= 0) continue;
        cellCosts[lr * winCols + lc] = (cellDistKm / speed) * 3600;
      }
    }

    // Check start/end passability
    if (cellCosts[lFromIdx] < 0 || cellCosts[lToIdx] < 0) {
      console.warn('[TerrainRaster] route start or end on impassable terrain');
      return null;
    }

    // A* search
    const gCost = new Float32Array(winRows * winCols).fill(Infinity);
    const parent = new Int32Array(winRows * winCols).fill(-1);
    const closed = new Uint8Array(winRows * winCols);

    gCost[lFromIdx] = 0;

    // Admissible heuristic: straight-line distance at max speed
    const maxSpeed = Math.max(...Object.values(speeds).filter((s): s is number => s > 0));
    const lToRow = toRow - winRowMin;
    const lToCol = toCol - winColMin;
    const h = (idx: number) => {
      const lr = (idx / winCols) | 0;
      const lc = idx % winCols;
      const dr = Math.abs(lr - lToRow);
      const dc = Math.abs(lc - lToCol);
      // ⚠ OCTILE, NOT MANHATTAN, AND THIS HAD TO CHANGE WITH THE NEIGHBOURS.
      // Manhattan counts a diagonal as two cells where eight-connected
      // movement crosses it in sqrt(2), so it OVERESTIMATES — which makes the
      // heuristic inadmissible and quietly costs A* its optimality. The
      // symptom would be slightly wrong paths, never an error.
      const straight = Math.max(dr, dc);
      const diagonal = Math.min(dr, dc);
      const cells = straight - diagonal + diagonal * DIAGONAL_COST;
      return ((cells * cellDistKm) / maxSpeed) * 3600;
    };

    const heap = new MinHeap();
    heap.push(h(lFromIdx), lFromIdx);

    let found = false;

    while (heap.size > 0) {
      const [, curIdx] = heap.pop()!;
      if (curIdx === lToIdx) { found = true; break; }
      if (closed[curIdx]) continue;
      closed[curIdx] = 1;

      const curRow = (curIdx / winCols) | 0;
      const curCol = curIdx % winCols;
      const curG = gCost[curIdx];

      for (let d = 0; d < DIR_R.length; d++) {
        const nr = curRow + DIR_R[d];
        const nc = curCol + DIR_C[d];
        if (nr < 0 || nr >= winRows || nc < 0 || nc >= winCols) continue;
        if (d >= DIAGONAL_FROM && !cornerOpen(cellCosts, winRows, winCols, curRow, curCol, d)) {
          continue;
        }
        const nIdx = nr * winCols + nc;
        if (closed[nIdx]) continue;
        const cc = cellCosts[nIdx];
        if (cc < 0) continue; // impassable

        const tentG = curG + (d >= DIAGONAL_FROM ? cc * DIAGONAL_COST : cc);
        if (tentG < gCost[nIdx]) {
          gCost[nIdx] = tentG;
          parent[nIdx] = curIdx;
          heap.push(tentG + h(nIdx), nIdx);
        }
      }
    }

    if (!found) {
      console.warn('[TerrainRaster] no route found');
      return null;
    }

    // Reconstruct path (local → global lat/lon)
    const waypoints: [number, number][] = [];
    let cur = lToIdx;
    while (cur !== -1) {
      const lr = (cur / winCols) | 0;
      const lc = cur % winCols;
      const gr = winRowMin + lr;
      const gc = winColMin + lc;
      waypoints.push([
        minLat + (numRows - 1 - gr + 0.5) * cellSize,
        minLon + (gc + 0.5) * cellSize,
      ]);
      cur = parent[cur];
    }
    waypoints.reverse();

    // Distance & duration
    let distanceKm = 0;
    for (let i = 1; i < waypoints.length; i++) {
      const dlat = waypoints[i][0] - waypoints[i - 1][0];
      const dlon = waypoints[i][1] - waypoints[i - 1][1];
      distanceKm += Math.sqrt(dlat * dlat + dlon * dlon) * 111.32;
    }

    console.log(
      `[TerrainRaster] route: ${waypoints.length} waypoints, ` +
      `${distanceKm.toFixed(1)} km, ${(gCost[lToIdx] / 60).toFixed(0)} min (${mobilityClass})`
    );

    return { waypoints, distanceKm, durationSec: gCost[lToIdx] };
  }

  /** Sector-based concave hull — same algorithm as the worker but runs on main thread (fast for grid data) */
  private _sectorHull(
    sourceLat: number,
    sourceLon: number,
    nodes: ReachableNode[],
    maxCostSec: number,
  ): GeoJsonFeature | null {
    if (nodes.length < 3) return null;

    const NUM_SECTORS = 72;
    const sectorMaxDist: number[] = new Array(NUM_SECTORS).fill(0);
    const sectorNode: (ReachableNode | null)[] = new Array(NUM_SECTORS).fill(null);

    for (const nd of nodes) {
      const dy = nd.lat - sourceLat;
      const dx = nd.lon - sourceLon;
      let angle = Math.atan2(dy, dx) * (180 / Math.PI);
      if (angle < 0) angle += 360;
      const sector = Math.floor(angle / (360 / NUM_SECTORS)) % NUM_SECTORS;
      const dist = dx * dx + dy * dy;
      if (dist > sectorMaxDist[sector]) {
        sectorMaxDist[sector] = dist;
        sectorNode[sector] = nd;
      }
    }

    // Interpolate empty sectors
    for (let i = 0; i < NUM_SECTORS; i++) {
      if (sectorNode[i] !== null) continue;
      let prevIdx = -1, nextIdx = -1;
      for (let d = 1; d < NUM_SECTORS; d++) {
        if (prevIdx === -1 && sectorNode[(i - d + NUM_SECTORS) % NUM_SECTORS] !== null) prevIdx = (i - d + NUM_SECTORS) % NUM_SECTORS;
        if (nextIdx === -1 && sectorNode[(i + d) % NUM_SECTORS] !== null) nextIdx = (i + d) % NUM_SECTORS;
        if (prevIdx !== -1 && nextIdx !== -1) break;
      }
      if (prevIdx !== -1 && nextIdx !== -1) {
        const p = sectorNode[prevIdx]!;
        const n = sectorNode[nextIdx]!;
        sectorNode[i] = { lat: (p.lat + n.lat) / 2, lon: (p.lon + n.lon) / 2, costSec: 0, globalNodeId: -1 };
      } else if (prevIdx !== -1) {
        sectorNode[i] = sectorNode[prevIdx];
      } else if (nextIdx !== -1) {
        sectorNode[i] = sectorNode[nextIdx];
      }
    }

    // Build ring [lon, lat] for GeoJSON
    const ring: [number, number][] = [];
    for (let i = 0; i < NUM_SECTORS; i++) {
      const s = sectorNode[i];
      if (s) ring.push([s.lon, s.lat]);
    }
    if (ring.length < 3) return null;

    // Chaikin smoothing (2 passes)
    let smoothed = ring;
    for (let pass = 0; pass < 2; pass++) {
      const next: [number, number][] = [];
      for (let i = 0; i < smoothed.length; i++) {
        const j = (i + 1) % smoothed.length;
        const [x0, y0] = smoothed[i];
        const [x1, y1] = smoothed[j];
        next.push([x0 * 0.75 + x1 * 0.25, y0 * 0.75 + y1 * 0.25]);
        next.push([x0 * 0.25 + x1 * 0.75, y0 * 0.25 + y1 * 0.75]);
      }
      smoothed = next;
    }

    // Close ring
    smoothed.push(smoothed[0]);

    return {
      type: 'Feature',
      properties: { maxCostSec },
      geometry: { type: 'Polygon', coordinates: [smoothed] },
    };
  }
}
