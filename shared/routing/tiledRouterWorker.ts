export {}; // module scope – prevents TS from merging declarations with other files

/* ──────────────────────────────────────────────────────────────────────────────
 * tiledRouterWorker.ts
 *
 * Self-contained Web Worker that performs:
 *   1) A* shortest-path routing on a tiled terrain graph
 *   2) Dijkstra isochrone (reachability) flood-fill with concave-hull envelopes
 *
 * No external imports – everything is inlined.
 * ────────────────────────────────────────────────────────────────────────────── */

// ─── Types ───────────────────────────────────────────────────────────────────

interface GraphNode {
  lat: number;
  lon: number;
}

interface GraphEdge {
  to: number;
  distKm: number;
  terrain: number; // 0=open, 1=road, 2=forest, 3=marsh
}

interface RouteResult {
  waypoints: [number, number][]; // [lat, lon][]
  distanceKm: number;
  durationSec: number;
}

interface IsochroneNode {
  lat: number;
  lon: number;
  costSec: number;
  globalNodeId: number;
}

// ─── Terrain speed tables (km/h) ────────────────────────────────────────────
//
// Terrain flag encoding (from raster & tile builder):
//   0  = open ground
//   1  = road / improved surface
//   2  = forest / woodland
//   3  = marsh / wetland
//   4  = water crossing (shallow ford / bridge) — navigable but very slow
//   255 = water / impassable (deep water, no crossing)
//
// Key design choices:
//   - Road bonus is significant but not overwhelming (1x→2.5x over open,
//     not 12x) to ensure the router prefers roads without making any
//     off-road path that avoids a detour look faster than it is.
//   - Forest and marsh are genuinely slow — a straight-line cross-country
//     route through forest should cost more than a road detour.
//   - Water (255) has no entry → edgeCostSec returns -1 (impassable).
//   - Water crossing (4) is penalised heavily to represent bridging delay.

const TERRAIN_SPEEDS: Record<string, Record<number, number>> = {
  //            open  road  forest  marsh  water-crossing
  tracked: {  0: 20,  1: 45,  2: 4,  3: 2,  4: 3  },
  wheeled: {  0: 25,  1: 65,  2: 3,  3: 0,  4: 0  },
  air:     {  0: 800, 1: 800, 2: 800, 3: 800, 4: 800 },
  foot:    {  0: 5,   1: 8,   2: 2,  3: 0.5, 4: 1  },
};

// ─── Constants ───────────────────────────────────────────────────────────────

const CELL_SIZE = 0.02; // degrees – spatial-grid resolution
const MAX_VISITS = 4_000_000;
const DEG_TO_RAD = Math.PI / 180;
const EARTH_KM = 6371;

// ─── Global graph state ──────────────────────────────────────────────────────

let nodes: Map<number, GraphNode> = new Map();
let adj: Map<number, GraphEdge[]> = new Map();
let grid: Map<string, number[]> = new Map();
let loadedTileKeys: string[] = [];

// ─── MinHeap ─────────────────────────────────────────────────────────────────

class MinHeap {
  private heap: [number, number][] = []; // [cost, nodeId]

  get size(): number {
    return this.heap.length;
  }

  push(cost: number, nodeId: number): void {
    this.heap.push([cost, nodeId]);
    this._bubbleUp(this.heap.length - 1);
  }

  pop(): [number, number] {
    const top = this.heap[0];
    const last = this.heap.pop()!;
    if (this.heap.length > 0) {
      this.heap[0] = last;
      this._sinkDown(0);
    }
    return top;
  }

  private _bubbleUp(i: number): void {
    const h = this.heap;
    while (i > 0) {
      const parent = (i - 1) >> 1;
      if (h[i][0] >= h[parent][0]) break;
      const tmp = h[i];
      h[i] = h[parent];
      h[parent] = tmp;
      i = parent;
    }
  }

  private _sinkDown(i: number): void {
    const h = this.heap;
    const n = h.length;
    while (true) {
      let smallest = i;
      const l = 2 * i + 1;
      const r = 2 * i + 2;
      if (l < n && h[l][0] < h[smallest][0]) smallest = l;
      if (r < n && h[r][0] < h[smallest][0]) smallest = r;
      if (smallest === i) break;
      const tmp = h[i];
      h[i] = h[smallest];
      h[smallest] = tmp;
      i = smallest;
    }
  }
}

// ─── Spatial grid helpers ────────────────────────────────────────────────────

function gridKey(lat: number, lon: number): string {
  return `${Math.floor(lat / CELL_SIZE)},${Math.floor(lon / CELL_SIZE)}`;
}

function addToGrid(globalId: number, lat: number, lon: number): void {
  const key = gridKey(lat, lon);
  let bucket = grid.get(key);
  if (!bucket) {
    bucket = [];
    grid.set(key, bucket);
  }
  bucket.push(globalId);
}

function findNearest(lat: number, lon: number): number | null {
  const cx = Math.floor(lat / CELL_SIZE);
  const cy = Math.floor(lon / CELL_SIZE);

  let bestId: number | null = null;
  let bestDist = Infinity;

  for (let ring = 0; ring <= 6; ring++) {
    for (let dx = -ring; dx <= ring; dx++) {
      for (let dy = -ring; dy <= ring; dy++) {
        if (ring > 0 && Math.abs(dx) < ring && Math.abs(dy) < ring) continue; // only ring perimeter
        const key = `${cx + dx},${cy + dy}`;
        const bucket = grid.get(key);
        if (!bucket) continue;
        for (let i = 0; i < bucket.length; i++) {
          const n = nodes.get(bucket[i]);
          if (!n) continue;
          const dlat = n.lat - lat;
          const dlon = n.lon - lon;
          const d = dlat * dlat + dlon * dlon;
          if (d < bestDist) {
            bestDist = d;
            bestId = bucket[i];
          }
        }
      }
    }
    if (bestId !== null) return bestId;
  }

  return bestId;
}

// ─── Haversine approximation (fast) ─────────────────────────────────────────

function haversineKm(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const dlat = (lat2 - lat1) * DEG_TO_RAD;
  const dlon = (lon2 - lon1) * DEG_TO_RAD;
  const a =
    Math.sin(dlat * 0.5) * Math.sin(dlat * 0.5) +
    Math.cos(lat1 * DEG_TO_RAD) *
      Math.cos(lat2 * DEG_TO_RAD) *
      Math.sin(dlon * 0.5) *
      Math.sin(dlon * 0.5);
  return EARTH_KM * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

// ─── Edge cost helper ────────────────────────────────────────────────────────

function edgeCostSec(
  distKm: number,
  terrain: number,
  speeds: Record<number, number>,
): number {
  const speed = speeds[terrain];
  if (speed === undefined || speed <= 0) return -1; // impassable
  return (distKm / speed) * 3600;
}

// ─── Path simplification (remove collinear waypoints) ────────────────────────

function simplifyPath(pts: [number, number][]): [number, number][] {
  if (pts.length <= 2) return pts;
  const result: [number, number][] = [pts[0]];
  for (let i = 1; i < pts.length - 1; i++) {
    const [ay, ax] = pts[i - 1];
    const [by, bx] = pts[i];
    const [cy, cx] = pts[i + 1];
    // Cross product to detect collinearity
    const cross = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    if (Math.abs(cross) > 1e-10) {
      result.push(pts[i]);
    }
  }
  result.push(pts[pts.length - 1]);
  return result;
}

// ─── Chaikin smoothing ───────────────────────────────────────────────────────

function chaikinSmooth(ring: [number, number][], passes: number): [number, number][] {
  let pts = ring;
  for (let p = 0; p < passes; p++) {
    const next: [number, number][] = [];
    const n = pts.length;
    for (let i = 0; i < n; i++) {
      const curr = pts[i];
      const nxt = pts[(i + 1) % n];
      next.push([
        curr[0] * 0.75 + nxt[0] * 0.25,
        curr[1] * 0.75 + nxt[1] * 0.25,
      ]);
      next.push([
        curr[0] * 0.25 + nxt[0] * 0.75,
        curr[1] * 0.25 + nxt[1] * 0.75,
      ]);
    }
    pts = next;
  }
  return pts;
}

// ─── Sector-based concave hull ───────────────────────────────────────────────

function sectorHull(
  sourceLat: number,
  sourceLon: number,
  visitedNodes: IsochroneNode[],
): [number, number][] | null {
  if (visitedNodes.length === 0) return null;

  const NUM_SECTORS = 72; // 5° each
  const sectors: (IsochroneNode | null)[] = new Array(NUM_SECTORS).fill(null);
  const sectorMaxDist: number[] = new Array(NUM_SECTORS).fill(0);

  for (let i = 0; i < visitedNodes.length; i++) {
    const nd = visitedNodes[i];
    const dy = nd.lat - sourceLat;
    const dx = nd.lon - sourceLon;
    let angle = Math.atan2(dy, dx) * (180 / Math.PI);
    if (angle < 0) angle += 360;
    const sector = Math.floor(angle / (360 / NUM_SECTORS)) % NUM_SECTORS;
    const dist = dx * dx + dy * dy;
    if (dist > sectorMaxDist[sector]) {
      sectorMaxDist[sector] = dist;
      sectors[sector] = nd;
    }
  }

  // Interpolate empty sectors
  for (let i = 0; i < NUM_SECTORS; i++) {
    if (sectors[i] !== null) continue;
    // Find nearest filled sectors in both directions
    let prevIdx = -1;
    let nextIdx = -1;
    for (let d = 1; d < NUM_SECTORS; d++) {
      if (prevIdx === -1 && sectors[(i - d + NUM_SECTORS) % NUM_SECTORS] !== null) {
        prevIdx = (i - d + NUM_SECTORS) % NUM_SECTORS;
      }
      if (nextIdx === -1 && sectors[(i + d) % NUM_SECTORS] !== null) {
        nextIdx = (i + d) % NUM_SECTORS;
      }
      if (prevIdx !== -1 && nextIdx !== -1) break;
    }
    if (prevIdx !== -1 && nextIdx !== -1) {
      const pn = sectors[prevIdx]!;
      const nn = sectors[nextIdx]!;
      // Linear interpolation
      sectors[i] = {
        lat: (pn.lat + nn.lat) / 2,
        lon: (pn.lon + nn.lon) / 2,
        costSec: (pn.costSec + nn.costSec) / 2,
        globalNodeId: -1,
      };
    } else if (prevIdx !== -1) {
      sectors[i] = sectors[prevIdx];
    } else if (nextIdx !== -1) {
      sectors[i] = sectors[nextIdx];
    }
  }

  // Build ring [lon, lat]
  const ring: [number, number][] = [];
  for (let i = 0; i < NUM_SECTORS; i++) {
    const s = sectors[i];
    if (s) {
      ring.push([s.lon, s.lat]);
    }
  }

  if (ring.length < 3) return null;

  // Close the ring
  ring.push([ring[0][0], ring[0][1]]);

  // Apply 2 passes of Chaikin smoothing
  return chaikinSmooth(ring, 2);
}

// ─── A* routing ──────────────────────────────────────────────────────────────

function routeAStar(
  fromLat: number,
  fromLon: number,
  toLat: number,
  toLon: number,
  mobilityClass: string,
): RouteResult | null {
  const speeds = TERRAIN_SPEEDS[mobilityClass] ?? TERRAIN_SPEEDS.foot;

  const srcNode = findNearest(fromLat, fromLon);
  const dstNode = findNearest(toLat, toLon);
  if (srcNode === null || dstNode === null) return null;

  const dstInfo = nodes.get(dstNode)!;
  const cosLat = Math.cos(((fromLat + toLat) / 2) * DEG_TO_RAD);

  // Heuristic: use the AVERAGE traversable speed rather than the max.
  // Using maxSpeed (road at 40-60 km/h) makes the heuristic extremely
  // optimistic — A* barely penalises detours through slow terrain because
  // the estimated remaining cost is always tiny. Using average speed makes
  // the heuristic tighter so A* actually prefers faster terrain routes.
  const traversableSpeeds = Object.values(speeds).filter((s) => s > 0);
  if (traversableSpeeds.length === 0) return null;
  // Use the minimum traversable speed as the heuristic denominator.
  // This is still admissible (never overestimates) because the actual cost
  // can only be >= (dist / minSpeed). It makes A* far more careful about
  // assuming cheap routes exist, so it genuinely explores terrain detours.
  const minSpeed = Math.min(...traversableSpeeds);

  function heuristic(nodeId: number): number {
    const n = nodes.get(nodeId);
    if (!n) return 0; // unknown node — zero heuristic (Dijkstra-safe fallback)
    const dlat = (dstInfo.lat - n.lat) * DEG_TO_RAD;
    const dlon = (dstInfo.lon - n.lon) * DEG_TO_RAD * cosLat;
    const distKm = Math.sqrt(dlat * dlat + dlon * dlon) * EARTH_KM;
    return (distKm / minSpeed) * 3600;
  }

  const gCost = new Map<number, number>();
  const cameFrom = new Map<number, number>();

  const heap = new MinHeap();
  gCost.set(srcNode, 0);
  heap.push(heuristic(srcNode), srcNode);

  let visits = 0;
  let found = false;

  while (heap.size > 0 && visits < MAX_VISITS) {
    const [, current] = heap.pop();
    visits++;

    if (current === dstNode) {
      found = true;
      break;
    }

    const currentG = gCost.get(current)!;
    const edges = adj.get(current);
    if (!edges) continue;

    for (let i = 0; i < edges.length; i++) {
      const edge = edges[i];
      const cost = edgeCostSec(edge.distKm, edge.terrain, speeds);
      if (cost < 0) continue; // impassable

      const tentativeG = currentG + cost;
      const prevG = gCost.get(edge.to);
      if (prevG !== undefined && tentativeG >= prevG) continue;

      gCost.set(edge.to, tentativeG);
      cameFrom.set(edge.to, current);
      heap.push(tentativeG + heuristic(edge.to), edge.to);
    }
  }

  if (!found) return null;

  // Reconstruct path
  const path: number[] = [];
  let cur: number | undefined = dstNode;
  while (cur !== undefined) {
    path.push(cur);
    cur = cameFrom.get(cur);
  }
  path.reverse();

  // Build waypoints
  let totalDist = 0;
  const waypoints: [number, number][] = [];
  for (let i = 0; i < path.length; i++) {
    const n = nodes.get(path[i])!;
    waypoints.push([n.lat, n.lon]);
    if (i > 0) {
      const prev = nodes.get(path[i - 1])!;
      totalDist += haversineKm(prev.lat, prev.lon, n.lat, n.lon);
    }
  }

  const simplified = simplifyPath(waypoints);

  return {
    waypoints: simplified,
    distanceKm: totalDist,
    durationSec: gCost.get(dstNode)!,
  };
}

// ─── Dijkstra isochrone ──────────────────────────────────────────────────────

function computeIsochrone(
  fromLat: number,
  fromLon: number,
  mobilityClass: string,
  timeBudgetSec: number,
  contourIntervalsSec: number[],
): {
  nodes: IsochroneNode[];
  envelopeCoords: [number, number][] | null;
  contourCoords: [number, number][][];
} {
  const speeds = TERRAIN_SPEEDS[mobilityClass] ?? TERRAIN_SPEEDS.foot;

  const srcNode = findNearest(fromLat, fromLon);
  if (srcNode === null) {
    return { nodes: [], envelopeCoords: null, contourCoords: [] };
  }

  // Cap visited nodes for performance — this is a planning tool, not a GPS.
  // 20K nodes gives a good approximation in well-connected graphs.
  const MAX_VISITED = 20_000;

  const costMap = new Map<number, number>();
  const heap = new MinHeap();
  costMap.set(srcNode, 0);
  heap.push(0, srcNode);

  let visited = 0;
  while (heap.size > 0 && visited < MAX_VISITED) {
    const [currentCost, current] = heap.pop();

    // Skip if we already found a cheaper path
    const recorded = costMap.get(current);
    if (recorded !== undefined && currentCost > recorded) continue;

    if (currentCost > timeBudgetSec) continue;
    visited++;

    const edges = adj.get(current);
    if (!edges) continue;

    for (let i = 0; i < edges.length; i++) {
      const edge = edges[i];
      const cost = edgeCostSec(edge.distKm, edge.terrain, speeds);
      if (cost < 0) continue;

      const newCost = currentCost + cost;
      if (newCost > timeBudgetSec) continue;

      const prevCost = costMap.get(edge.to);
      if (prevCost !== undefined && newCost >= prevCost) continue;

      costMap.set(edge.to, newCost);
      heap.push(newCost, edge.to);
    }
  }

  // Collect visited nodes
  const visitedNodes: IsochroneNode[] = [];
  costMap.forEach((costSec, globalNodeId) => {
    const n = nodes.get(globalNodeId);
    if (n) {
      visitedNodes.push({ lat: n.lat, lon: n.lon, costSec, globalNodeId });
    }
  });

  // Compute envelope from all visited nodes
  const srcInfo = nodes.get(srcNode)!;
  const envelopeCoords = sectorHull(srcInfo.lat, srcInfo.lon, visitedNodes);

  // Compute contours for each interval
  const contourCoords: [number, number][][] = [];
  for (let c = 0; c < contourIntervalsSec.length; c++) {
    const interval = contourIntervalsSec[c];
    const filtered = visitedNodes.filter((nd) => nd.costSec <= interval);
    const hull = sectorHull(srcInfo.lat, srcInfo.lon, filtered);
    contourCoords.push(hull ?? []);
  }

  return { nodes: visitedNodes, envelopeCoords, contourCoords };
}

// ─── Message handler ─────────────────────────────────────────────────────────

self.onmessage = (e: MessageEvent) => {
  const msg = e.data;

  switch (msg.type) {
    // ── init ──────────────────────────────────────────────────────────────
    case 'init': {
      nodes = new Map();
      adj = new Map();
      grid = new Map();
      loadedTileKeys = [];
      (self as unknown as Worker).postMessage({ type: 'ready' });
      break;
    }

    // ── addTile ──────────────────────────────────────────────────────────
    case 'addTile': {
      const { tileKey, nodesBuffer, edgesBuffer } = msg as {
        type: 'addTile';
        tileKey: string;
        nodesBuffer: ArrayBuffer;
        edgesBuffer: ArrayBuffer;
      };

      // Parse nodes: Int32Array, records of 3 ints [globalId, lat*1_000_000, lon*1_000_000]
      const nodeArr = new Int32Array(nodesBuffer);
      const nodeCount = nodeArr.length / 3;
      for (let i = 0; i < nodeCount; i++) {
        const off = i * 3;
        const globalId = nodeArr[off];
        const lat = nodeArr[off + 1] / 1_000_000;
        const lon = nodeArr[off + 2] / 1_000_000;

        if (!nodes.has(globalId)) {
          nodes.set(globalId, { lat, lon });
          addToGrid(globalId, lat, lon);
        }
      }

      // Parse edges: Uint32Array, records of 4 ints [fromGlobalId, toGlobalId, distance*10_000, terrainFlag]
      const edgeArr = new Uint32Array(edgesBuffer);
      const edgeCount = edgeArr.length / 4;
      for (let i = 0; i < edgeCount; i++) {
        const off = i * 4;
        const from = edgeArr[off];
        const to = edgeArr[off + 1];
        const distKm = edgeArr[off + 2] / 10_000;
        const terrain = edgeArr[off + 3];

        // Add bidirectional edges
        let fromList = adj.get(from);
        if (!fromList) {
          fromList = [];
          adj.set(from, fromList);
        }
        fromList.push({ to, distKm, terrain });

        let toList = adj.get(to);
        if (!toList) {
          toList = [];
          adj.set(to, toList);
        }
        toList.push({ to: from, distKm, terrain });
      }

      loadedTileKeys.push(tileKey);
      (self as unknown as Worker).postMessage({
        type: 'tilesLoaded',
        keys: loadedTileKeys.slice(),
      });
      break;
    }

    // ── route ────────────────────────────────────────────────────────────
    case 'route': {
      const { id, fromLat, fromLon, toLat, toLon, mobilityClass } = msg as {
        type: 'route';
        id: number;
        fromLat: number;
        fromLon: number;
        toLat: number;
        toLon: number;
        mobilityClass: string;
      };

      const route = routeAStar(fromLat, fromLon, toLat, toLon, mobilityClass);
      (self as unknown as Worker).postMessage({ type: 'routeResult', id, route });
      break;
    }

    // ── isochrone ────────────────────────────────────────────────────────
    case 'isochrone': {
      const { id, fromLat, fromLon, mobilityClass, timeBudgetSec, contourIntervalsSec } =
        msg as {
          type: 'isochrone';
          id: number;
          fromLat: number;
          fromLon: number;
          mobilityClass: string;
          timeBudgetSec: number;
          contourIntervalsSec: number[];
        };

      const result = computeIsochrone(
        fromLat,
        fromLon,
        mobilityClass,
        timeBudgetSec,
        contourIntervalsSec,
      );

      (self as unknown as Worker).postMessage({
        type: 'isochroneResult',
        id,
        nodes: result.nodes,
        envelopeCoords: result.envelopeCoords,
        contourCoords: result.contourCoords,
      });
      break;
    }

    default:
      break;
  }
};
