// ── realtime/world/airspace.ts ───────────────────────────────────────────
// Airspace (W7, W11): guns and aircraft sharing the sky. A shell's path from
// gun to target climbs to its maximum ordinate; an own aircraft below that,
// near the gun–target line, is in the way. The fires cell is asked before the
// first round goes (airspaceConflict): hold the guns, shift the aircraft, or
// accept the risk — and an accepted risk can cost an aircraft.

import type { LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import { hasRecord, recordFor } from "../catalogue";
import type { WorldCtx } from "./context";
import { wfig } from "./figures";
import { distanceM } from "./geo";
import type { Entity } from "./types";

const ROTOR_OR_FIXED = new Set(["fixedWing", "rotary"]);

/** Where a trajectory peaks: about a quarter of its range for a gun, capped. */
export function maxOrdinateM(rangeM: number): number {
  return Math.min(wfig("world/airspace.ORDINATE_CAP_M"), rangeM * wfig("world/airspace.ORDINATE_SHARE"));
}

/** Distance from a point to the segment a→b, m (flat approximation, good to tens of kilometres). */
function offLineM(p: LatLng, a: LatLng, b: LatLng): number {
  const k = 111_320;
  const cos = Math.cos((a.lat * Math.PI) / 180);
  const ax = 0, ay = 0;
  const bx = (b.lng - a.lng) * k * cos, by = (b.lat - a.lat) * k;
  const px = (p.lng - a.lng) * k * cos, py = (p.lat - a.lat) * k;
  const len = bx * bx + by * by;
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - ax) * bx + (py - ay) * by) / len));
  return Math.hypot(px - bx * t, py - by * t);
}

/** Own manned aircraft in the gun–target corridor, below the trajectory's peak. */
export function aircraftInCorridor(c: WorldCtx, side: Side, from: LatLng, to: LatLng): Entity[] {
  const ceiling = maxOrdinateM(distanceM(from, to));
  const width = wfig("world/airspace.CORRIDOR_M");
  return Object.values(c.world.entities)
    .filter((e) => e.owner === side && e.domain === "air" && e.status === "active" && hasRecord(e.archetype) && ROTOR_OR_FIXED.has(recordFor(e.archetype).mover.kind) && recordFor(e.archetype).body.crew > 0)
    .filter((e) => e.altM <= ceiling && offLineM(e.pos, from, to) <= width)
    .sort((a, b) => a.id.localeCompare(b.id));
}
