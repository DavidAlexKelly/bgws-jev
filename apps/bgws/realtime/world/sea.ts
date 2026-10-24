// ── realtime/world/sea.ts ────────────────────────────────────────────────
// Water terrain (W8): where the sea is, and how deep. The sea is a set of
// polygons; a scenario without any is open ocean. Depth is declared from
// the distance to the shore — a continental shelf deepening steadily to the
// deep-water figure — until a bathymetry dataset (GEBCO) replaces it.

import type { LatLng } from "../../lib/board";
import type { TerrainSampler } from "../../lib/lineOfSight";
import { wfig } from "./figures";
import type { WorldState } from "./types";

const K = 111_320;

function inPolygon(p: LatLng, poly: LatLng[]): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const a = poly[i];
    const b = poly[j];
    if (a.lat > p.lat !== b.lat > p.lat && p.lng < ((b.lng - a.lng) * (p.lat - a.lat)) / (b.lat - a.lat) + a.lng) inside = !inside;
  }
  return inside;
}

function toSegmentM(p: LatLng, a: LatLng, b: LatLng): number {
  const cos = Math.cos((p.lat * Math.PI) / 180);
  const ax = (a.lng - p.lng) * K * cos, ay = (a.lat - p.lat) * K;
  const bx = (b.lng - p.lng) * K * cos, by = (b.lat - p.lat) * K;
  const dx = bx - ax, dy = by - ay;
  const len = dx * dx + dy * dy;
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, -(ax * dx + ay * dy) / len));
  return Math.hypot(ax + dx * t, ay + dy * t);
}

/** Is this point at sea? */
export function isWater(world: Pick<WorldState, "sea">, p: LatLng): boolean {
  const sea = world.sea;
  if (!sea || !sea.waters.length) return true;
  return sea.waters.some((w) => inPolygon(p, w));
}

/** Distance to the nearest shore, m (Infinity in open ocean; 0 on land). */
export function toShoreM(world: Pick<WorldState, "sea">, p: LatLng): number {
  const sea = world.sea;
  if (!sea || !sea.waters.length) return Infinity;
  if (!isWater(world, p)) return 0;
  let best = Infinity;
  for (const w of sea.waters) for (let i = 0; i < w.length; i++) best = Math.min(best, toSegmentM(p, w[i], w[(i + 1) % w.length]));
  return best;
}

/** Depth of water here, m: the shelf deepening from the shore, down to deep water. */
export function depthAt(world: Pick<WorldState, "sea">, p: LatLng): number {
  const shore = toShoreM(world, p);
  return Math.min(wfig("world/sea.DEEP_M"), shore * wfig("world/sea.SHELF_SLOPE"));
}

const WRAPPED = new WeakMap<TerrainSampler, WeakMap<object, TerrainSampler>>();

/** The ground as the world sees it: generated or loaded terrain on land, sea level and open water at sea. */
export function seaTerrain(terrain: TerrainSampler, sea: WorldState["sea"]): TerrainSampler {
  if (!sea || !sea.waters.length) return terrain;
  let bySea = WRAPPED.get(terrain);
  if (!bySea) WRAPPED.set(terrain, (bySea = new WeakMap()));
  const hit = bySea.get(sea);
  if (hit) return hit;
  const world = { sea };
  const wrapped: TerrainSampler = {
    groundHeightM: (p) => (isWater(world, p) ? 0 : Math.max(1, terrain.groundHeightM(p))),
    classify: (p) => (isWater(world, p) ? "water" : terrain.classify(p)),
  };
  bySea.set(sea, wrapped);
  return wrapped;
}
