// ── bgws/realtime/engine/timing.ts ─────────────────────────────────────────
// Turning the rules' per-turn rates into per-second ones.
//
// The fire and sighting tables were written for 15-minute turns. Real time
// needs them per tick, and there are two kinds of rule to convert:
//
//   CHANCES    something that happens with probability p in a turn happens
//              with probability 1 − (1 − p)^(dt / turnS) in a tick of dt.
//   ACTIONS    a shot is an action, not a chance. A unit that is engaging
//              fires every `shotIntervalS`. The fire table is rolled as it
//              is, but its RESULT is scaled: a hit only costs strength with
//              probability hits × lethalityPerTurn × shotIntervalS / turnS.
//              Every shot, misses included, adds suppression.
//
// ⚠ `lethalityPerTurn` IS THE CALIBRATION KNOB. The table was written for one
// result per 15-minute turn; rolled every 30 s unscaled it broke units in a
// couple of minutes, which is what made them withdraw for ever. The chance a
// hit does damage is also scaled by range (fire.ts): about 2.4× at 300 m,
// 0.6× at 2.5 km. At 1, identical forces fight for 20-25 sim-minutes and end
// on a breakpoint — see the balance and decisiveness checks in realtime.test.ts.

import type { RtTiming } from "./types";

/** Closer than this, a moving unit halts: it has run into the enemy. */
export const CLOSE_CONTACT_M = 300;
/** An assault keeps closing until this close. */
export const ASSAULT_CONTACT_M = 150;
/** Closer than this, with a line of sight, nobody fails to see a unit (halted target). */
export const AUTO_SIGHT_M = 500;
/** A moving target is seen without a roll from this far. */
export const AUTO_SIGHT_MOVING_M = 800;
/** A settled, hull-down target in cover is only certain to be seen this close. */
export const AUTO_SIGHT_HIDDEN_M = 300;

/** Suppression at or above this lowers accuracy. */
export const SUPPRESSED_AT = 25;
/** Suppression at or above this pins: no advancing. */
export const PINNED_AT = 60;
/** Suppression fades by this much per second once fire stops... */
export const SUPPRESSION_DECAY_PER_S = 2;
/** ...after this long without an incoming shot. */
export const SUPPRESSION_GRACE_S = 10;
/** Pinned this long, and a unit tests its nerve. */
export const PINNED_TEST_S = 60;

/** Break-test thresholds, as fractions of strength lost (Dupuy: attackers ~20%, defenders ~40%). */
export const ATTACKER_BREAK_AT = 0.2;
export const DEFENDER_BREAK_AT = 0.4;
export const BREAK_STEP = 0.2;
/** A side is beaten when this much of its strength is destroyed or broken. */
export const SIDE_BREAKPOINT = 0.5;

/** An HQ within this range steadies nerves and speeds rallying. */
export const HQ_RADIUS_M = 1500;
/** Cover this close is worth dashing for in the react-to-contact drill. */
export const DRILL_COVER_M = 150;
/** Bounding overwatch: how far a bound goes, and how long the cover halt lasts. */
export const BOUND_M = 300;
export const BOUND_COVER_S = 40;
/** Still for this long, a unit has settled into its position. */
export const SETTLE_S = 30;
/** A unit engaging for this long is reviewed: "ineffective" if it has done nothing, "review" if it has. */
export const REVIEW_S = 180;
/** Decisions each unit remembers. */
export const HISTORY_LENGTH = 4;

export const DEFAULT_TIMING: RtTiming = {
  tickS: 1,
  turnS: 15 * 60,
  shotIntervalS: 30,
  lethalityPerTurn: 1,
  sightingIntervalS: 30,
  contactMemoryS: 120,
  rallyCheckS: 60,
  idleS: 75,
  reportDelayS: 15,
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
