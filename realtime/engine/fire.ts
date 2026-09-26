// ── bgws/realtime/engine/fire.ts ───────────────────────────────────────────
// Whether a round hits, in real time. One place, used by the engine to roll
// fire and by the options to state the odds, so the two cannot disagree.
//
// NOT THE TURN GAME'S FIRE TABLE. That table gives one result for a whole
// troop's fifteen-minute turn, from its combat strength; it knows one range
// effect (-1 beyond half maximum range) and nothing of fire control. Rolled
// every few seconds it reported point-blank rounds as near misses two times
// in three. Real time uses a per-round model instead, as tank engagement
// models do (see docs/REALTIME_REALISM.md §2):
//
//   every fit vehicle in the firing unit fires a round per aimed shot;
//   each round hits with a chance from range (a modern fire-control
//   system's first-round figures) times what degrades it — moving, cover,
//   suppression; each hit then goes through lethality.ts: the face it
//   strikes, penetration there, whether the vehicle is knocked out.
//
// Every figure here is DECLARED: the L7 tables carry no accuracy data, and
// optics_class has no data dictionary yet. They are the place to plug that
// data in when it arrives.

import type { RtTiming } from "./types";

/** DECLARED. Chance one aimed round hits a stationary, exposed vehicle, by range: [metres, chance]. */
const HIT_BY_RANGE: readonly [number, number][] = [
  [300, 0.97],
  [500, 0.95],
  [1000, 0.9],
  [1500, 0.8],
  [2000, 0.65],
  [2500, 0.5],
  [3000, 0.4],
];

/** DECLARED. What degrades a round's chance of hitting, as multipliers. */
export const HIT_FACTORS = {
  /** Firing on the move, even stabilised. */
  firerMoving: 0.7,
  /** A crossing or moving target. */
  targetMoving: 0.85,
  /** In woods or towns, or hull-down: less of it to hit. */
  targetCovered: 0.6,
  /** Rounds coming in: the crew is ducking. */
  firerSuppressed: 0.8,
  /** Pinned: barely looking out. */
  firerPinned: 0.5,
  /** Shaken: firing wild. */
  firerShaken: 0.6,
} as const;

/** The chance a round hits a stationary, exposed target at this range. */
export function hitAtRange(rangeM: number): number {
  const points = HIT_BY_RANGE;
  if (rangeM <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i += 1) {
    const [x1, y1] = points[i];
    if (rangeM <= x1) {
      const [x0, y0] = points[i - 1];
      return y0 + ((y1 - y0) * (rangeM - x0)) / (x1 - x0);
    }
  }
  return points[points.length - 1][1];
}

export interface HitConditions {
  firerMoving?: boolean;
  targetMoving?: boolean;
  targetCovered?: boolean;
  firerSuppressed?: boolean;
  firerPinned?: boolean;
  firerShaken?: boolean;
}

/** The chance one round hits, at this range and in these conditions (× `strikeScale`, 1 unless a test says otherwise). */
export function hitChance(rangeM: number, conditions: HitConditions, timing: RtTiming): number {
  let p = hitAtRange(rangeM);
  for (const [key, factor] of Object.entries(HIT_FACTORS)) {
    if (conditions[key as keyof HitConditions]) p *= factor;
  }
  return Math.min(1, Math.max(0, p * timing.strikeScale));
}
