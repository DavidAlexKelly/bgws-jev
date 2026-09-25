// ── bgws/lib/movePlan.ts ───────────────────────────────────────────────────
// The seam between "where a commander would like to go" and "where the ground
// lets it get to this turn".
//
// WHY THIS FILE EXISTS
// --------------------
// `movement.ts` has had the rule since it was written: an allowance per Move
// Type per terrain class, spent in FRACTIONS of a turn, with a missing or zero
// entry meaning IMPASSABLE rather than slow. `terrainAdapter.ts` has had a
// table to spend. `ruleset.ts` has carried that table on every RuleSet and an
// accessor to read it.
//
// Nothing called any of it. `optionsFor` offered a destination 40% of the way
// towards the enemy and the loop teleported the element there, over water, up
// a cliff, through a forest, at the same rate in every case. The rule existed;
// no sequence of play could reach it. That is this project's recurring bug and
// it is the one the wiring guard was built to catch — it did not catch this
// one because `movement` is a table, not a modifier, and the guard only sweeps
// modifiers.
//
// WHAT THIS DOES AND DOES NOT DO
// ------------------------------
// It samples the STRAIGHT LINE to the requested destination, classifies each
// sample, and spends the allowance along it. So a move stops where the turn
// runs out and stops at the edge of ground the Move Type cannot enter.
//
// It does NOT route AROUND an obstacle. Going around is a different decision —
// it needs the A* router in shared/routing, which needs tiles, which needs the
// network, which the harness does not have. The straight-line check is the
// half that can be made deterministic and offline, and it is the half that
// stops an illegal move. `legsFromWaypoints` in terrainAdapter.ts is already
// the entry point for the routed version when a caller has a router.

import { distanceM, metresPerDegreeLon, type LatLng } from "./board";
import type { MoveType } from "../data/profiles";
import type { TerrainSampler } from "./lineOfSight";
import {
  consumeAllowance,
  interpolate,
  type AllowanceTable,
  type MoveOutcome,
  type MoveResult,
  type RouteLeg,
  type TerrainClass,
} from "./movement";

/**
 * Metres between terrain samples along a route.
 *
 * The procedural ground's cover cells are 250 m and the DEM tiles are coarser
 * still, so 100 m samples the terrain finer than either engine can resolve.
 * Smaller would cost time and tell us nothing new.
 */
export const SAMPLE_STEP_M = 100;

/**
 * Hard cap on samples per route, so a silly destination cannot make option
 * generation quadratic. 64 × 100 m is 6.4 km, and the board is 10 km — beyond
 * the cap the legs simply get longer, which costs accuracy on a move nothing
 * can afford anyway.
 */
const MAX_LEGS = 64;

export interface MovePlan {
  /** Where the element actually ends up. Never further than the rules allow. */
  destination: LatLng;
  /** Distance actually covered. Zero means the move is not worth offering. */
  distanceM: number;
  outcome: MoveOutcome;
  /** Set when the route ran into ground this Move Type cannot enter. */
  blockedBy?: TerrainClass;
  /** Fraction of the turn's movement spent, 0-1. */
  allowanceSpent: number;
  legs: RouteLeg[];
  /**
   * Set when the element had to go round rather than straight, and by how
   * much. Reported so a summary can say so — a bound that goes 30° off the
   * axis is a different decision from one that goes straight, and a commander
   * choosing from summaries should be able to see which it is getting.
   */
  detouredDeg?: number;
  /** What stopped the direct line, when a detour was taken to avoid it. */
  detouredAround?: TerrainClass;
}

/**
 * Classify the straight line from one point to another into legs.
 *
 * Each leg is classified at its MIDPOINT, matching `legsFromWaypoints` — the
 * cheapest reading that cannot classify a whole leg by the terrain it happens
 * to start on.
 */
export function legsAlong(
  terrain: TerrainSampler,
  from: LatLng,
  to: LatLng,
  stepM: number = SAMPLE_STEP_M,
): RouteLeg[] {
  const total = distanceM(from, to);
  if (total <= 0) return [];

  const count = Math.min(MAX_LEGS, Math.max(1, Math.ceil(total / stepM)));
  const legs: RouteLeg[] = [];
  let cursor = from;

  for (let i = 1; i <= count; i += 1) {
    const next = interpolate(from, to, i / count);
    const midpoint = interpolate(cursor, next, 0.5);
    legs.push({
      terrain: terrain.classify(midpoint),
      distanceM: distanceM(cursor, next),
      to: next,
    });
    cursor = next;
  }

  return legs;
}

/**
 * NOTHING IS TRAPPED BY THE GROUND IT IS STANDING ON.
 *
 * A truck generated inside a marsh, or left there by a map that changed under
 * it, would otherwise have every route blocked at its first leg and would sit
 * there for the rest of the game — an immobile element that no rule put there
 * and no rule can release. That is a generator artefact wearing a rule's
 * clothes, and it would be found as "the movement check is broken".
 *
 * So the LEADING legs in the terrain the element occupies are re-priced at the
 * WORST rate this Move Type can manage anywhere. Getting out of the bog is as
 * expensive as the worst going it can handle, and it is possible. Re-entering
 * the same terrain later in the route is still blocked — this buys an exit,
 * not a licence.
 */
function allowEscape(
  legs: readonly RouteLeg[],
  standing: TerrainClass,
  allowances: Partial<Record<TerrainClass, number>>,
): RouteLeg[] {
  if ((allowances[standing] ?? 0) > 0) return [...legs];

  const passable = Object.values(allowances).filter((metres) => metres > 0);
  if (passable.length === 0) return [...legs];
  const worst = Math.min(...passable);
  const worstClass = (Object.keys(allowances) as TerrainClass[]).find(
    (klass) => allowances[klass] === worst,
  );
  if (!worstClass) return [...legs];

  const out = [...legs];
  for (let i = 0; i < out.length && out[i].terrain === standing; i += 1) {
    out[i] = { ...out[i], terrain: worstClass };
  }
  return out;
}

/**
 * AN ELEMENT NEVER ENDS ITS MOVE INSIDE GROUND IT CANNOT ENTER.
 *
 * ⚠ THIS WAS A REAL BUG, FOUND BY A TEST THAT ASSERTED THE OBVIOUS.
 *
 * Legs are priced by their MIDPOINT — the reading that stops a whole leg being
 * classified by the terrain it happens to start on — and the sample grid does
 * not line up with the edge of a bog. So a 98 m leg from 491 m to 589 m has
 * its midpoint at 540 m, is priced as open ground, and DELIVERS THE ELEMENT
 * 39 m INSIDE A MARSH IT HAS NO ALLOWANCE FOR. The route was legal by the
 * metre and illegal by the map.
 *
 * Rather than re-price legs by their end (pessimistic: every leg that clips a
 * wood becomes a wood) the end point is walked back along the last leg until
 * it stands on ground the Move Type can occupy. Ten fixed steps, so it is
 * deterministic and a replay is exact, and it can only ever shorten a move.
 */
function stopOutsideImpassable(
  result: MoveResult,
  terrain: TerrainSampler,
  allowances: Partial<Record<TerrainClass, number>>,
  standing: TerrainClass,
  start: LatLng,
): MoveResult {
  const canOccupy = (point: LatLng): boolean => {
    const klass = terrain.classify(point);
    // Where it already stands is always occupiable: it is standing there.
    return klass === standing || (allowances[klass] ?? 0) > 0;
  };

  if (result.distanceM <= 0 || canOccupy(result.end)) return result;

  // Back along the final leg, in tenths, to the last point it may occupy.
  const legs = result.legs;
  const lastLegStart =
    legs.length > 1 ? legs[legs.length - 2].to : start;

  for (let step = 1; step <= 10; step += 1) {
    const fraction = 1 - step / 10;
    const candidate = interpolate(lastLegStart, result.end, fraction);
    if (!canOccupy(candidate)) continue;

    const shortfall = distanceM(candidate, result.end);
    const trimmed = legs.slice(0, -1);
    if (fraction > 0) {
      const last = legs[legs.length - 1];
      trimmed.push({ ...last, distanceM: Math.max(0, last.distanceM - shortfall), to: candidate });
    }
    return {
      ...result,
      // Stopping short of impassable ground IS being blocked by it, whatever
      // the accumulator thought while it was pricing metres.
      outcome: "blocked",
      blockedBy: result.blockedBy ?? terrain.classify(result.end),
      distanceM: Math.max(0, result.distanceM - shortfall),
      end: candidate,
      legs: trimmed,
    };
  }

  // Nowhere on the last leg is occupiable: the move does not happen.
  return {
    outcome: "blocked",
    allowanceSpent: 0,
    distanceM: 0,
    end: start,
    legs: [],
    blockedBy: terrain.classify(result.end),
  };
}

export interface MovePlanRequest {
  terrain: TerrainSampler;
  moveType: MoveType;
  table: AllowanceTable;
  from: LatLng;
  to: LatLng;
  stepM?: number;
}

/**
 * How far along the line to `to` this Move Type gets in one turn.
 *
 * The requested route is first clipped to the BEST allowance this Move Type
 * has on any terrain, because nothing can travel further than that in a turn
 * whatever the going is. It is a pure optimisation — it saves sampling ground
 * the element could never reach — and it is safe precisely because the cap is
 * the best case.
 */
export function planMove(request: MovePlanRequest): MovePlan {
  const { terrain, moveType, table, from, to, stepM } = request;
  const allowances = table[moveType] ?? {};
  const metres = Object.values(allowances);
  const best = metres.length > 0 ? Math.max(...metres) : 0;

  const requested = distanceM(from, to);
  if (requested <= 0 || best <= 0) {
    return {
      destination: from,
      distanceM: 0,
      outcome: best <= 0 ? "blocked" : "completed",
      allowanceSpent: 0,
      legs: [],
    };
  }

  const reach = Math.min(requested, best);
  const target = reach < requested ? interpolate(from, to, reach / requested) : to;
  const standing = terrain.classify(from);
  const legs = allowEscape(legsAlong(terrain, from, target, stepM), standing, allowances);
  const raw = consumeAllowance(moveType, legs, table, from);
  const result = stopOutsideImpassable(raw, terrain, allowances, standing, from);

  return {
    destination: result.end,
    distanceM: result.distanceM,
    // A route that was clipped to the allowance cap and then completed is not
    // "completed" from the commander's point of view: the element wanted to go
    // further and the turn stopped it.
    outcome:
      result.outcome === "completed" && reach < requested ? "exhausted" : result.outcome,
    blockedBy: result.blockedBy,
    allowanceSpent: result.allowanceSpent,
    legs: result.legs,
  };
}

/**
 * Bearings to try, in order, when the direct line will not do.
 *
 * ⚠ WITHOUT THIS, IMPASSABLE GROUND MEANS PARALYSIS RATHER THAN A DETOUR.
 *
 * The straight-line check alone made a blocked element simply hold: 9 of its
 * move options came back blocked and it sat there. Mean game length went from
 * 12 turns to 22 with wet ground on the board, almost all of it elements
 * standing still in front of a bog that a real one would have driven round.
 *
 * A full A* around the obstacle needs the tiled router, which needs the
 * network, which the harness does not have. Five fixed bearings is the coarse
 * version of the same idea and it is deterministic, offline, and enough to
 * find a way past a patch: try straight, then 30° and 60° either side, and
 * take whichever candidate ends up closest to where the element wanted to be.
 *
 * Left of centre before right at equal merit, so a tie is broken the same way
 * every time — a replay has to be exact.
 */
const DETOUR_BEARINGS_DEG: readonly number[] = [0, -30, 30, -60, 60];

/**
 * A direct plan this poor is worth a second look. Expressed as a fraction of
 * what was asked, so it scales with the size of the bound.
 */
const DETOUR_THRESHOLD = 0.5;

/** Rotate `to` about `from` by `deg`, in the local metre frame. */
function rotateAbout(from: LatLng, to: LatLng, deg: number): LatLng {
  if (deg === 0) return to;
  const metresPerLng = metresPerDegreeLon(from.lat);
  const east = (to.lng - from.lng) * metresPerLng;
  const north = (to.lat - from.lat) * METRES_PER_DEGREE_LAT;
  const radians = (deg * Math.PI) / 180;
  const rotatedEast = east * Math.cos(radians) + north * Math.sin(radians);
  const rotatedNorth = -east * Math.sin(radians) + north * Math.cos(radians);
  return {
    lat: from.lat + rotatedNorth / METRES_PER_DEGREE_LAT,
    lng: from.lng + rotatedEast / metresPerLng,
  };
}

const METRES_PER_DEGREE_LAT = 111_320;

/**
 * The best move towards `to` this turn, going round what it cannot cross.
 *
 * The direct line is tried first and kept whenever it is good enough, so the
 * usual case costs exactly one plan. Only a move that is blocked, or that gets
 * less than half way, pays for the detour search.
 */
export function planMoveAround(request: MovePlanRequest): MovePlan {
  const direct = planMove(request);
  const wanted = distanceM(request.from, request.to);
  if (wanted <= 0) return direct;
  if (direct.outcome !== "blocked" && direct.distanceM >= wanted * DETOUR_THRESHOLD) {
    return direct;
  }

  let best = direct;
  // How close the candidate gets to where the element actually wanted to be.
  // Progress towards the objective is the thing being maximised, not distance
  // covered — a long run sideways is not a better move than a short one
  // forwards.
  let bestGap = distanceM(direct.destination, request.to);

  for (const deg of DETOUR_BEARINGS_DEG) {
    if (deg === 0) continue;
    const candidate = planMove({ ...request, to: rotateAbout(request.from, request.to, deg) });
    if (candidate.distanceM <= 0) continue;
    const gap = distanceM(candidate.destination, request.to);
    if (gap < bestGap - 1) {
      best = { ...candidate, detouredDeg: deg, detouredAround: direct.blockedBy };
      bestGap = gap;
    }
  }

  return best;
}

/** Terrain in plain words, for an option summary a commander has to read. */
export function describeTerrain(terrain: TerrainClass): string {
  switch (terrain) {
    case "woodsLight":
      return "woodland";
    case "woodsThick":
      return "thick woods";
    case "water":
      return "water";
    case "marsh":
      return "marsh";
    case "steep":
      return "steep ground";
    case "urban":
      return "built-up ground";
    case "crops":
      return "standing crops";
    case "road":
      return "road";
    case "open":
      return "open ground";
    default:
      return terrain;
  }
}

/**
 * The clause a truncated move adds to its summary.
 *
 * A commander — human, heuristic or model — chooses from summaries, so a move
 * that will not arrive has to SAY it will not arrive. Silently offering "X
 * advances on Y" for a bound that covers 300 m of a 2 km gap is how a model
 * ends up believing it has closed when it has not.
 */
export function describeMovePlan(plan: MovePlan): string {
  const metres = `${Math.round(plan.distanceM).toLocaleString("en-GB")} m`;
  const round =
    plan.detouredDeg === undefined
      ? ""
      : `, round the ${describeTerrain(plan.detouredAround ?? "marsh")}`;
  if (plan.outcome === "blocked") {
    return `${metres}${round}, halted by ${describeTerrain(plan.blockedBy ?? "water")}`;
  }
  if (plan.outcome === "exhausted") {
    return `${metres}${round}, as far as one turn allows`;
  }
  return `${metres}${round}`;
}
