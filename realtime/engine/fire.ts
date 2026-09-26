// ── bgws/realtime/engine/fire.ts ───────────────────────────────────────────
// What range does to a shot in real time. One place, used by the engine to
// roll fire and by the options to state the odds, so the two cannot disagree.
//
// The turn game's table knows one range effect: -1 beyond half the weapon's
// maximum range. So a tank at 300 m shot no better than at 1,500 m, and with
// every hit scaled down for real time, point-blank fights read as endless
// misses. Two things fix it, neither touching the turn game:
//
//   RANGE BANDS   a modifier on the roll: +2 within 500 m, +1 within 1 km.
//                 Beyond half maximum range the table's own -1 still applies.
//   LETHALITY     the chance a hit does damage scales with range: about 4×
//                 more at 300 m than at 2.5 km. The chance of a kill given a
//                 hit rises steeply as range closes (see the tank engagement
//                 models in docs/REALTIME_REALISM.md); a hit at 3 km is often a
//                 glancing or non-penetrating one.

import type { Modifier } from "../../rules/events";
import type { RtTiming } from "./types";

/** Range bands: [out to, modifier]. */
const RANGE_BANDS: readonly [number, number][] = [
  [500, 2],
  [1000, 1],
];

/** Lethality by range, [metres, factor], interpolated between points. */
const LETHALITY_BY_RANGE: readonly [number, number][] = [
  [300, 2.4],
  [500, 2.0],
  [1000, 1.3],
  [1500, 0.9],
  [2500, 0.6],
  [3500, 0.5],
];

/** The real-time range modifier for a shot, as the fire table's `extraModifiers`. */
export function rangeModifiers(rangeM: number): Modifier[] {
  const band = RANGE_BANDS.find(([limit]) => rangeM <= limit);
  return band ? [{ source: band[1] >= 2 ? "pointBlank" : "closeRange", value: band[1] }] : [];
}

/** How much more (or less) lethal a hit is at this range than on average. */
export function lethalityFactor(rangeM: number): number {
  const points = LETHALITY_BY_RANGE;
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

/** The chance one fire-table hit does damage, at this range. */
export function damagePerHit(rangeM: number, timing: RtTiming): number {
  return Math.min(1, ((timing.lethalityPerTurn * timing.shotIntervalS) / timing.turnS) * lethalityFactor(rangeM));
}

/** A shot's outcome, in words a reader can trust: a "hit" that did nothing is not a hit. */
export function shotLabel(hits: number, suppressed: boolean, damaged: boolean): string {
  if (damaged) return hits > 1 ? `struck ${hits}×, damaged` : "struck, damaged";
  if (hits > 0) return hits > 1 ? `struck ${hits}×, no damage` : "struck, no damage";
  return suppressed ? "near miss, suppressed" : "missed";
}
