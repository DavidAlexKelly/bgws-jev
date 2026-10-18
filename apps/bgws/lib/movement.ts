// ── bgws/lib/movement.ts ───────────────────────────────────────────────────
// Maximum Allowable Distance: how far a Force Element gets this turn.
//
// THE MECHANISM IS HERE; THE NUMBERS ARE NOT.
//
// BGWS gives each Move Type an allowance per terrain type, and a move that
// crosses several terrains spends a FRACTION of each allowance rather than a
// distance: the rulebook's own example is a Foot FE going 1,000 m along a road
// (half of its 2,000 m road allowance) and then 500 m across open ground (half
// of its 1,000 m open allowance), which exactly exhausts it.
//
// That table is the TERRAIN EFFECTS TABLE on Player Aid 3 — 9.1 names it, and
// also bounds it: "The Maximum Allowable Distance is a figure between 1 and 6.
// Each value represents 1,000 metres." So the rulebook does tell us the SCALE
// even though it withholds the table, and any allowance outside 1,000-6,000 m
// a turn is wrong on the rulebook's own terms. (Earlier comments here said
// Player Aid 2; that is the Morale Checks and Orders table, not this one.)
//
// The table itself is not in the Core Rulebook, so it is not in
// this file either — `AllowanceTable` is a parameter with no default. A
// scenario supplies it, a test supplies its own, and nothing here invents a
// number that a training audience might later mistake for doctrine.
//
// Pure: no map, no router, no async. A caller routes on whatever terrain
// engine it has (this repo has TerrainRaster and the tiled router in
// shared/routing) and hands the legs in.

import type { MoveType } from "../data/profiles";
import { distanceM, type LatLng } from "./board";

/**
 * Terrain classes BGWS distinguishes for movement and line of sight.
 *
 * Deliberately coarse: the game's table has a row per class, and a finer
 * classification would have to be collapsed into these anyway.
 */
export type TerrainClass =
  | "road"
  | "open"
  | "crops"
  | "woodsLight"
  | "woodsThick"
  | "urban"
  | "marsh"
  | "water"
  | "steep";

/**
 * Metres of that terrain a Move Type may cross in one turn.
 *
 * A missing entry, or zero, means impassable — which is a different thing from
 * slow, and is why this is not a speed.
 */
export type AllowanceTable = Record<MoveType, Partial<Record<TerrainClass, number>>>;

/** One stretch of a route, already classified by the caller's terrain engine. */
export interface RouteLeg {
  terrain: TerrainClass;
  distanceM: number;
  /** Where this leg ends. Lets a truncated move report a real position. */
  to: LatLng;
}

export type MoveOutcome =
  | "completed"
  | "exhausted"
  | "blocked";

export interface MoveResult {
  outcome: MoveOutcome;
  /** Fraction of the turn's allowance spent, 0-1. */
  allowanceSpent: number;
  /** Distance actually travelled. */
  distanceM: number;
  /** Where the FE ends up. */
  end: LatLng;
  /** Legs fully or partly travelled, truncated to what the allowance bought. */
  legs: RouteLeg[];
  /** Set when the move stopped at impassable ground. */
  blockedBy?: TerrainClass;
}

/**
 * Spend a Move Type's allowance along a classified route.
 *
 * The accumulator is in FRACTIONS of the turn's allowance, not in metres,
 * because the allowance differs per terrain and the whole point of the rule is
 * that mixing terrain mixes the rates. A leg that would overrun is cut
 * proportionally and the FE stops there.
 */
export function consumeAllowance(
  moveType: MoveType,
  legs: readonly RouteLeg[],
  table: AllowanceTable,
  start: LatLng,
): MoveResult {
  const allowances = table[moveType] ?? {};
  const travelled: RouteLeg[] = [];
  let spent = 0;
  let distance = 0;
  let end = start;

  for (const leg of legs) {
    const allowance = allowances[leg.terrain] ?? 0;
    if (allowance <= 0) {
      return {
        outcome: "blocked",
        allowanceSpent: spent,
        distanceM: distance,
        end,
        legs: travelled,
        blockedBy: leg.terrain,
      };
    }

    const cost = leg.distanceM / allowance;
    if (spent + cost <= 1) {
      spent += cost;
      distance += leg.distanceM;
      end = leg.to;
      travelled.push(leg);
      continue;
    }

    // Part of this leg is affordable. Cut it where the allowance runs out.
    const affordable = (1 - spent) * allowance;
    if (affordable > 0) {
      const fraction = affordable / leg.distanceM;
      const cutEnd = interpolate(end, leg.to, fraction);
      travelled.push({ ...leg, distanceM: affordable, to: cutEnd });
      distance += affordable;
      end = cutEnd;
    }
    return {
      outcome: "exhausted",
      allowanceSpent: 1,
      distanceM: distance,
      end,
      legs: travelled,
    };
  }

  return {
    outcome: "completed",
    allowanceSpent: spent,
    distanceM: distance,
    end,
    legs: travelled,
  };
}

/**
 * A point a fraction of the way from one position to another.
 *
 * Linear in degrees, which is wrong over hundreds of kilometres and
 * indistinguishable from correct inside a 10 km sheet.
 */
export function interpolate(from: LatLng, to: LatLng, fraction: number): LatLng {
  const t = Math.max(0, Math.min(1, fraction));
  return {
    lat: from.lat + (to.lat - from.lat) * t,
    lng: from.lng + (to.lng - from.lng) * t,
  };
}

/**
 * Column movement (BGWS 9.1): faster, and only legal on a road for the whole
 * move with no enemy able to see any part of it.
 *
 * The visibility half is the caller's: it has the sight lines. This answers
 * the half that is geometry.
 */
export function isRoadOnly(legs: readonly RouteLeg[]): boolean {
  return legs.length > 0 && legs.every((leg) => leg.terrain === "road");
}

/** Straight-line distance of a leg list, for reporting. */
export function routeLength(start: LatLng, legs: readonly RouteLeg[]): number {
  let total = 0;
  let cursor = start;
  for (const leg of legs) {
    total += distanceM(cursor, leg.to);
    cursor = leg.to;
  }
  return total;
}
