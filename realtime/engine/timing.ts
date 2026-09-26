// ── bgws/realtime/engine/timing.ts ─────────────────────────────────────────
// Turning the rules' per-turn rates into per-second ones.
//
// The fire and sighting tables were written for 15-minute turns. Real time
// needs them per tick, and there are two kinds of rule to convert:
//
//   CHANCES    something that happens with probability p in a turn happens
//              with probability 1 − (1 − p)^(dt / turnS) in a tick of dt.
//   ACTIONS    a shot is an action, not a chance. A unit that is engaging
//              resolves one shot every `engagementCycleS`, using the fire
//              table exactly as it is.
//
// ⚠ `engagementCycleS` IS THE CALIBRATION KNOB. At 300 s a unit gets three
// shots in the time a turn-based unit gets one, which makes real time
// bloodier per simulated minute; a game is about how the trade goes, not how
// long it took, so what matters is the ratio of fire to movement. The
// headless runner exists so that ratio can be tuned against the turn game —
// see realtime/engine/calibration.test.ts.

import type { RtTiming } from "./types";

/** Closer than this, a moving unit halts: it has run into the enemy. */
export const CLOSE_CONTACT_M = 300;
/** Closer than this, with a line of sight, nobody fails to see a unit. */
export const AUTO_SIGHT_M = 500;

export const DEFAULT_TIMING: RtTiming = {
  tickS: 1,
  turnS: 15 * 60,
  engagementCycleS: 300,
  sightingIntervalS: 30,
  contactMemoryS: 120,
  recoveryS: 180,
  coalesceS: 3,
  cooldownS: 20,
  // A crew takes longer to react the worse it is: 15 s for a conscript
  // (TQ 1), 5 s at TQ 6 and above. Fixed, so a replay is exact.
  reactionS: (troopQuality) => Math.max(5, Math.min(15, 17 - 2 * troopQuality)),
  maxDurationS: 90 * 60,
};

/** A per-turn probability, as a per-tick one. */
export function perTick(pPerTurn: number, timing: RtTiming): number {
  const p = Math.min(1, Math.max(0, pPerTurn));
  return 1 - Math.pow(1 - p, timing.tickS / timing.turnS);
}

/** Metres a unit covers in one tick, from a per-turn allowance. */
export function metresPerTick(allowancePerTurnM: number, timing: RtTiming): number {
  return (allowancePerTurnM / timing.turnS) * timing.tickS;
}

/** "12:34" for a number of simulated seconds. */
export function clock(seconds: number): string {
  const whole = Math.floor(seconds);
  const minutes = Math.floor(whole / 60);
  const secs = whole % 60;
  return `${minutes}:${String(secs).padStart(2, "0")}`;
}
