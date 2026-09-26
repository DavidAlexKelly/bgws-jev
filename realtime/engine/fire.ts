// ── bgws/realtime/engine/fire.ts ───────────────────────────────────────────
// Whether a round hits, in real time: an ERROR BUDGET, as engagement models
// (AMSAA, Janus, the DTIC tank models in docs/REALTIME_REALISM.md §2) do it.
// One place, used by the engine to roll fire and by the options to state the
// odds, so the two cannot disagree.
//
// A round is aimed at the centre of what the gunner can see. Where it lands
// is off by the sum of independent errors:
//
//   bias          boresight, ammunition lot, fire-control calibration   (mils)
//   dispersion    round-to-round scatter of that gun and round           (mils)
//   firer motion  what the stabiliser does not take out                  (mils)
//   range error   how far off the range estimate is; matters in the
//                 vertical, more for a slow round with a curved flight   (metres)
//   lead error    against a moving target: the error in its estimated
//                 speed × the round's TIME OF FLIGHT (range ÷ muzzle
//                 velocity) — which is why a sabot beats a HEAT round    (metres)
//
// Angular errors grow with range (1 mil ≈ 1 m per km). Against them stands the
// target's PRESENTED size: wider side-on than head-on, only a turret when
// hull-down, part of it when in woods. The chance of a hit is the chance the
// errors land inside it (two normal distributions over a rectangle).
//
// FROM THE DATA: muzzle velocity and sustained rate of fire per weapon (L7
// bgws_capability_profile), the face presented (geometry, lethality.ts),
// hull-down (DEM). DECLARED, and marked: the fire-control errors (one
// "modern" class until optics_class has a dictionary), default muzzle
// velocities and rates where a weapon has none, target sizes by target class
// (the length column mixes gun-forward and hull-only conventions, and
// signature_class is derived from it, so neither is used), and guided
// missiles, which are not ballistic at all.

import type { Capability, ForceElement } from "../../lib/state";
import type { Aspect } from "./lethality";
import type { RtTiming } from "./types";

/** DECLARED. One "modern fire control" class: laser rangefinder, ballistic computer, stabiliser. */
const FIRE_CONTROL = {
  biasMil: 0.2,
  /** Laser rangefinder, 1σ. */
  rangeErrorM: 10,
  /** Error in the target's estimated speed, 1σ, with a lead-computing sight. */
  speedErrorMs: 1.0,
  /** Firing on the move, after stabilisation. */
  firerMovingMil: 0.5,
};

/** DECLARED. Round-to-round dispersion, 1σ. */
const DISPERSION_MIL = { ke: 0.2, ce: 0.35, automatic: 1.5 };

/** DECLARED. Muzzle velocity when the weapon has none in the data. */
const DEFAULT_MUZZLE_VELOCITY = { ke: 1600, ce: 900, automatic: 850, other: 800 };

/** DECLARED. Sustained aimed rounds a minute when the weapon has none. */
const DEFAULT_ROF = { gun: 6, missile: 2, automatic: 200 };

/**
 * An automatic weapon fires bursts, not aimed rounds: one burst every this
 * many seconds counts as one "round" against its dispersion. DECLARED.
 */
const BURST_CYCLE_S = 6;
/** Faster than this is automatic fire. */
const AUTOMATIC_ROF = 30;

/** DECLARED. Guided missiles: hit chance from the guidance, not ballistics. */
const GUIDED = { hit: 0.9, vsMoving: 0.9, firerMoving: 0.3 };

/** DECLARED. What degrades aim: multipliers on the angular error. */
const NERVE = { suppressed: 1.4, pinned: 2.2, shaken: 1.8 };

/** DECLARED. Presented size by target class, metres. `exposed` is what shows over a crest when hull-down. */
const TARGET_SIZE: Record<string, { width: number; length: number; height: number; exposed: number }> = {
  // A hull-only MBT/IFV: the length is the hull, not gun-forward.
  armoured_vehicle: { width: 3.6, length: 7.5, height: 2.4, exposed: 1.0 },
  soft_skin: { width: 2.5, length: 7.0, height: 2.8, exposed: 1.2 },
  // A section in fire positions: spread out and low.
  foot: { width: 10, length: 10, height: 0.5, exposed: 0.4 },
};
/** DECLARED. Woods or buildings hide part of what is there. */
const COVER_EXPOSED = { width: 0.7, height: 0.5 };

const G = 9.81;

/** A surface-to-surface missile rather than a ballistic round. */
export function isGuided(weapon: Capability): boolean {
  return weapon.kind === "atm";
}

/** Automatic fire: bursts rather than aimed rounds. */
export function isAutomatic(weapon: Capability): boolean {
  return (weapon.rofSustained ?? (weapon.kind === "apers" ? DEFAULT_ROF.automatic : 0)) > AUTOMATIC_ROF;
}

/** Muzzle velocity: the data's, else a declared default for the kind of round. */
export function muzzleVelocity(weapon: Capability): number {
  if (weapon.muzzleVelocityMs) return weapon.muzzleVelocityMs;
  if (isAutomatic(weapon)) return DEFAULT_MUZZLE_VELOCITY.automatic;
  if (weapon.munition === "ce" || weapon.munition === "ceTandem") return DEFAULT_MUZZLE_VELOCITY.ce;
  if (weapon.kind === "atk") return DEFAULT_MUZZLE_VELOCITY.ke;
  return DEFAULT_MUZZLE_VELOCITY.other;
}

/**
 * Seconds between aimed shots for one vehicle with this weapon: from its
 * sustained rate of fire (L7), a burst cycle for automatic weapons, or a
 * declared default. `floorS` (timing.shotIntervalS) is the least time an
 * engagement cycle — acquire, lay, fire, observe — can take.
 */
export function aimedIntervalS(weapon: Capability, floorS: number): number {
  if (isAutomatic(weapon)) return Math.max(floorS, BURST_CYCLE_S);
  const rof = weapon.rofSustained ?? (isGuided(weapon) ? DEFAULT_ROF.missile : DEFAULT_ROF.gun);
  return Math.max(floorS, 60 / Math.max(0.1, rof));
}

/** P(|N(0, σ)| < half) — the chance an error stays within half a dimension. */
function within(half: number, sigma: number): number {
  if (sigma <= 0) return 1;
  return erf(half / (sigma * Math.SQRT2));
}

/** Abramowitz & Stegun 7.1.26: |error| < 1.5e-7. */
function erf(x: number): number {
  const sign = x < 0 ? -1 : 1;
  const t = 1 / (1 + 0.3275911 * Math.abs(x));
  const y =
    1 -
    ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) *
      t *
      Math.exp(-x * x);
  return sign * y;
}

export interface HitConditions {
  /** The face the round meets: sets the width presented. */
  aspect?: Aspect;
  firerMoving?: boolean;
  targetMoving?: boolean;
  /** In woods or buildings. */
  targetInCover?: boolean;
  /** Behind a crest: only the turret shows. */
  targetHullDown?: boolean;
  firerSuppressed?: boolean;
  firerPinned?: boolean;
  firerShaken?: boolean;
}

/** The size of what the gunner can see to aim at. */
export function presented(target: ForceElement, c: HitConditions): { width: number; height: number } {
  const size = TARGET_SIZE[target.targetClass] ?? TARGET_SIZE.armoured_vehicle;
  let width = c.aspect === "side" ? size.length : c.aspect === "roof" ? size.length : size.width;
  let height = c.aspect === "roof" ? size.width : c.targetHullDown ? size.exposed : size.height;
  if (c.targetInCover) {
    width *= COVER_EXPOSED.width;
    height *= COVER_EXPOSED.height;
  }
  return { width, height };
}

/** The error budget, broken down: for the odds shown to Jev and for tests. */
export function errorBudget(weapon: Capability, rangeM: number, c: HitConditions) {
  const automatic = isAutomatic(weapon);
  const shaped = weapon.munition === "ce" || weapon.munition === "ceTandem";
  const dispersion = automatic ? DISPERSION_MIL.automatic : shaped ? DISPERSION_MIL.ce : DISPERSION_MIL.ke;
  const nerve = c.firerPinned ? NERVE.pinned : c.firerShaken ? NERVE.shaken : c.firerSuppressed ? NERVE.suppressed : 1;
  const angularMil =
    Math.sqrt(FIRE_CONTROL.biasMil ** 2 + dispersion ** 2 + (c.firerMoving ? FIRE_CONTROL.firerMovingMil ** 2 : 0)) *
    nerve;
  const angularM = (angularMil * rangeM) / 1000;
  const velocity = muzzleVelocity(weapon);
  const timeOfFlightS = rangeM / velocity;
  // A range error moves the fall of shot by (drop's rate of change with range) × error.
  const rangeM_ = (FIRE_CONTROL.rangeErrorM * G * rangeM) / (velocity * velocity);
  const leadM = c.targetMoving ? FIRE_CONTROL.speedErrorMs * timeOfFlightS * nerve : 0;
  return {
    angularMil,
    timeOfFlightS,
    sigmaX: Math.sqrt(angularM ** 2 + leadM ** 2),
    sigmaY: Math.sqrt(angularM ** 2 + rangeM_ ** 2),
  };
}

/** The chance one round (or burst) from `weapon` hits `target` at this range, in these conditions. */
export function hitChance(
  weapon: Capability,
  target: ForceElement,
  rangeM: number,
  c: HitConditions,
  timing: RtTiming,
): number {
  let p: number;
  if (isGuided(weapon)) {
    // Guided: the missile flies to the target, so range hardly matters inside
    // its envelope — but it must be guided from a halt, and cover still hides.
    p = GUIDED.hit * (c.targetMoving ? GUIDED.vsMoving : 1) * (c.firerMoving ? GUIDED.firerMoving : 1);
    if (c.targetInCover) p *= COVER_EXPOSED.height;
    if (c.firerPinned) p *= 1 / NERVE.pinned;
    else if (c.firerShaken) p *= 1 / NERVE.shaken;
    else if (c.firerSuppressed) p *= 1 / NERVE.suppressed;
  } else {
    const { width, height } = presented(target, c);
    const { sigmaX, sigmaY } = errorBudget(weapon, rangeM, c);
    p = within(width / 2, sigmaX) * within(height / 2, sigmaY);
  }
  return Math.min(1, Math.max(0, p * timing.strikeScale));
}
