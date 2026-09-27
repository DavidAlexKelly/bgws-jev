// ── bgws/realtime/engine/detection.ts ──────────────────────────────────────
// Spotting an enemy, second by second.
//
// A crew scanning a sector finds a target at some RATE, not on a schedule:
// the chance of spotting it in the next second depends on how far away it
// is, whether it is moving or has just fired, whether it is in cover or
// hull-down, and whether the crew itself is moving or ducking. That is the
// search model engagement simulations use (detection as a rate, falling with
// the square of range), in place of the turn game's once-a-turn sighting
// table on a stagger.
//
// Once spotted, a target in sight is TRACKED — no further roll — and it is
// identified (sighting "full") once close enough to tell what it is.
//
// Every figure is DECLARED: the L7 tables carry no sensor data, and
// optics_class and signature_class have no dictionary. This is where they
// plug in when they do.

/** DECLARED. A halted, exposed vehicle at 1 km is found in 10 s on average. */
const BASE_RATE_PER_S = 0.1;
const REFERENCE_M = 1000;
/** DECLARED. Inside this, nobody fails to see what is in plain view. */
export const CERTAIN_M = 150;
/** DECLARED. Inside this, a spotted target is identified (type known), not only "a contact". */
export const IDENTIFY_M = 2000;

/** DECLARED. What makes a target easier or harder to find: multipliers on the rate. */
const FACTORS = {
  targetMoving: 3,
  /** Muzzle flash, dust and smoke: a vehicle that has just fired gives itself away. */
  targetFired: 5,
  targetInCover: 0.3,
  targetHullDown: 0.4,
  /** Still for a while: camouflaged, engine off. */
  targetSettled: 0.7,
  /** Small and low: a section of infantry. */
  targetOnFoot: 0.3,
  /** Buttoned up and bouncing. */
  observerMoving: 0.5,
  observerSuppressed: 0.5,
  observerPinned: 0.2,
  /** Searching the bearing it was fired on from (D3): spotting doubled there. */
  observerSearching: 2,
  /** Still and watching, not firing: observing, or waiting for a trigger. */
  observerWatching: 1.5,
} as const;

export type DetectionConditions = Partial<Record<keyof typeof FACTORS, boolean>>;

/** Detections per second at this range, in these conditions. */
export function detectionRate(rangeM: number, conditions: DetectionConditions): number {
  let rate = BASE_RATE_PER_S * (REFERENCE_M / Math.max(1, rangeM)) ** 2;
  for (const [key, factor] of Object.entries(FACTORS)) {
    if (conditions[key as keyof DetectionConditions]) rate *= factor;
  }
  return rate;
}

/** The chance of spotting it within `dtS` seconds. */
export function detectChance(rangeM: number, conditions: DetectionConditions, dtS: number): number {
  if (rangeM <= CERTAIN_M) return 1;
  return 1 - Math.exp(-detectionRate(rangeM, conditions) * dtS);
}
