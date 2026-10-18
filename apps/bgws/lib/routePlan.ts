// ── bgws/lib/routePlan.ts ──────────────────────────────────────────────────
// A route is a plan that outlives a turn.
//
// WHAT WAS WRONG WITH ONE-TURN MOVEMENT
// -------------------------------------
// `movePlan` answers "how far does this element get THIS turn towards that
// point". That is the right question for a bound in contact and the wrong one
// for a march: an element ordered to an objective 8 km away was re-deciding
// its direction every turn from scratch, with no memory of where it was going
// and no way to commit to a line of advance. It could not be given a route
// round a lake, because a route round a lake only makes sense if it is still
// being followed three turns later.
//
// So a route is state: a polyline the element is committed to, spent a turn's
// allowance at a time. Plan far, move what the ground allows, keep the rest
// for next turn.
//
// AND IT IS ABANDONED ON CONTACT. A march is a plan made in the absence of the
// enemy; the moment the element sights one, the plan is stale by definition
// and the commander should be choosing again. That is `routeInterrupted`.

import { distanceM, type LatLng } from "./board";
import type { MoveType } from "../data/profiles";
import type { TerrainSampler } from "./lineOfSight";
import { consumeAllowance, type AllowanceTable, type MoveOutcome, type RouteLeg } from "./movement";
import { legsAlong, planMoveAround, SAMPLE_STEP_M } from "./movePlan";

/** A plan an element is following, which survives between turns. */
export interface PlannedRoute {
  /** Where it is trying to get to. Kept so a re-plan can aim at the same place. */
  goal: LatLng;
  /** What it is for, in words a commander can read. */
  label: string;
  /** The rest of the route, nearest first. Consumed as the element advances. */
  waypoints: LatLng[];
  /** Turn the route was planned on, for the log and for the map's legend. */
  plannedOnTurn: number;
  /** Which planner produced it — a real router, or bearings. Never guessed at. */
  planner: "raster" | "bearings";
}

/**
 * Something that can find a way from here to there.
 *
 * Two implementations, deliberately: the raster's A* when there is a raster
 * (the browser), and a bearing search when there is not (the harness, which
 * plays tens of thousands of games with no network). Both return the same
 * thing, so the rules never learn which one they got.
 */
export interface RoutePlanner {
  readonly kind: "raster" | "bearings";
  /** Waypoints from `from` to `to`, excluding `from`. Null if it cannot route. */
  plan(from: LatLng, to: LatLng, moveType: MoveType): LatLng[] | null;
}

/** What one turn of marching did to a route. */
export interface RouteProgress {
  end: LatLng;
  distanceM: number;
  outcome: MoveOutcome;
  /** The route as it stands after this turn. Empty when the goal is reached. */
  remaining: LatLng[];
  legs: RouteLeg[];
}

/**
 * Spend one turn's allowance along a route.
 *
 * The route is followed waypoint by waypoint, classifying the ground as it
 * goes, and stops where the allowance runs out — mid-leg if necessary, with
 * the part-finished leg left in `remaining` so next turn picks it up exactly
 * where this one stopped. Nothing is rounded to a waypoint: an element that
 * can afford 2,250 m of a 6 km route moves 2,250 m.
 */
export function advanceAlongRoute(
  terrain: TerrainSampler,
  moveType: MoveType,
  table: AllowanceTable,
  from: LatLng,
  waypoints: readonly LatLng[],
  stepM: number = SAMPLE_STEP_M,
): RouteProgress {
  const legs: RouteLeg[] = [];
  let cursor = from;
  for (const waypoint of waypoints) {
    legs.push(...legsAlong(terrain, cursor, waypoint, stepM));
    cursor = waypoint;
  }

  if (legs.length === 0) {
    return { end: from, distanceM: 0, outcome: "completed", remaining: [], legs: [] };
  }

  const result = consumeAllowance(moveType, legs, table, from);
  const travelled = result.distanceM;

  // Which waypoints are behind us now? Walk the original list, subtracting the
  // distance covered, and keep whatever the element has not reached.
  const remaining: LatLng[] = [];
  let spent = travelled;
  let previous = from;
  for (const waypoint of waypoints) {
    const leg = distanceM(previous, waypoint);
    if (spent >= leg) {
      spent -= leg;
      previous = waypoint;
      continue;
    }
    remaining.push(waypoint);
    previous = waypoint;
  }

  return {
    end: result.end,
    distanceM: travelled,
    outcome: result.outcome,
    remaining,
    legs: result.legs,
  };
}

/**
 * A planner that asks the terrain raster's A* for a real route.
 *
 * Takes the narrowest possible slice of TerrainRaster — one synchronous
 * method — so this file needs no dataset, no network and no browser to be
 * tested, and so a different router can be substituted without touching BGWS.
 */
export interface RasterRouter {
  route(
    fromLat: number,
    fromLon: number,
    toLat: number,
    toLon: number,
    mobilityClass: "tracked" | "wheeled" | "air" | "foot",
  ): { waypoints: [number, number][] } | null;
  /** Optional, and worth having: it lets an unreachable goal be snapped. */
  isPassable?(lat: number, lon: number): boolean;
}

/**
 * Where to look for a passable cell when the goal itself is not one.
 *
 * ⚠ AN OBJECTIVE ON A RIVER IS A REAL CASE, NOT AN EDGE CASE. A* refuses to
 * plan a route whose endpoint is impassable, and an objective placed on water,
 * in a marsh or on the wrong side of a bank makes every march to it fail — the
 * element then falls back to blind bounds and nobody can see why. Red did
 * exactly this in the first game played on the raster: blue marched, red did
 * not, because red's objective sat on a river.
 *
 * Rings of eight, nearest first, so the snap is deterministic and always the
 * closest usable ground rather than whichever cell a search happened to reach.
 */
const SNAP_RINGS_M = [250, 500, 1000, 2000];
const SNAP_BEARINGS = 8;
const METRES_PER_DEGREE_LAT_SNAP = 111_320;

function nearbyPassable(
  router: RasterRouter,
  goal: LatLng,
): LatLng | null {
  if (!router.isPassable) return null;
  if (router.isPassable(goal.lat, goal.lng)) return goal;

  const metresPerLng = METRES_PER_DEGREE_LAT_SNAP * Math.cos((goal.lat * Math.PI) / 180);
  for (const radius of SNAP_RINGS_M) {
    for (let i = 0; i < SNAP_BEARINGS; i += 1) {
      const angle = (i / SNAP_BEARINGS) * 2 * Math.PI;
      const candidate = {
        lat: goal.lat + (radius * Math.cos(angle)) / METRES_PER_DEGREE_LAT_SNAP,
        lng: goal.lng + (radius * Math.sin(angle)) / metresPerLng,
      };
      if (router.isPassable(candidate.lat, candidate.lng)) return candidate;
    }
  }
  return null;
}

export function rasterRoutePlanner(
  router: RasterRouter,
  mobilityFor: (moveType: MoveType) => "tracked" | "wheeled" | "air" | "foot",
  simplify?: (waypoints: [number, number][]) => [number, number][],
): RoutePlanner {
  return {
    kind: "raster",
    plan(from, to, moveType) {
      // An objective on a river cannot be routed to, and refusing to march is
      // the worst of the available answers: march to the nearest ground that
      // can be stood on instead, and let the next turn's re-plan close the gap.
      const target = nearbyPassable(router, to) ?? to;
      const found = router.route(
        from.lat,
        from.lng,
        target.lat,
        target.lng,
        mobilityFor(moveType),
      );
      if (!found || found.waypoints.length === 0) return null;
      const points = simplify ? simplify(found.waypoints) : found.waypoints;
      const waypoints = points.map(([lat, lng]) => ({ lat, lng }));

      // A* reconstructs its path from the goal backwards. The router reverses
      // it before returning, and this is the belt to that braces: a route
      // handed back end-first would march the element away from its objective,
      // which is the kind of bug that looks like bad pathfinding for a week.
      if (
        waypoints.length > 1 &&
        distanceM(waypoints[0], target) < distanceM(waypoints[waypoints.length - 1], target)
      ) {
        waypoints.reverse();
      }
      // A* starts at the cell the element is standing in, which is where it
      // already is; carrying it would add a zero-length first leg.
      if (waypoints.length > 1 && distanceM(waypoints[0], from) < 1) waypoints.shift();
      return waypoints.length > 0 ? waypoints : null;
    },
  };
}

/** Give up rather than search forever; 20 hops is more than a board is wide. */
const MAX_HOPS = 20;

/**
 * The offline planner: repeated one-turn bounds, each allowed to go round what
 * it cannot cross.
 *
 * ⚠ THIS IS NOT A*, AND THE DIFFERENCE IS REAL. It is greedy: it takes the
 * best of five bearings at each hop and cannot back out of a dead end, so a
 * deep bay or a long ridge will trap it where a real search would go round.
 * It exists because the harness plays tens of thousands of games with no
 * network and therefore no raster, and because a deterministic mediocre route
 * is worth more to a batch than an excellent one it cannot have.
 *
 * `planner: "bearings"` travels with the route so nothing downstream — the
 * map, the log, a report — has to guess which of the two it is looking at.
 */
export function bearingRoutePlanner(
  terrain: TerrainSampler,
  table: AllowanceTable,
): RoutePlanner {
  return {
    kind: "bearings",
    plan(from, to, moveType) {
      const waypoints: LatLng[] = [];
      let cursor = from;

      for (let hop = 0; hop < MAX_HOPS; hop += 1) {
        const remaining = distanceM(cursor, to);
        if (remaining < 1) break;

        const plan = planMoveAround({
          terrain,
          moveType,
          table,
          from: cursor,
          to,
        });

        // Nowhere to go from here: keep what we have rather than inventing a
        // leg the element could not walk.
        if (plan.distanceM <= 0) break;

        waypoints.push(plan.destination);
        cursor = plan.destination;

        // Arrived — or as close as the planner can get.
        if (distanceM(cursor, to) < 1) break;
      }

      if (waypoints.length === 0) return null;
      return waypoints;
    },
  };
}

/**
 * Should this element abandon its route?
 *
 * A march is a plan made in the absence of the enemy. Sighting one makes it
 * stale by definition: the commander is now choosing between fighting,
 * skirting and pressing on, and offering "continue along the route" as though
 * nothing had happened would hide that choice rather than pose it.
 *
 * Also true when the goal has been reached, and when the route has run out of
 * waypoints — both of which mean the plan is finished rather than interrupted.
 */
export function routeInterrupted(
  route: PlannedRoute | undefined,
  options: { sightedEnemy: boolean; position: LatLng; arrivedWithinM: number },
): boolean {
  if (!route) return false;
  if (options.sightedEnemy) return true;
  if (route.waypoints.length === 0) return true;
  return distanceM(options.position, route.goal) <= options.arrivedWithinM;
}
