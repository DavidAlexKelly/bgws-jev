/**
 * A route is a plan that outlives a turn.
 *
 * The distinction these tests hold on to: a BOUND asks how far towards a point
 * an element gets this turn, and can therefore never go round anything,
 * because going round costs ground in the turn it happens. A ROUTE is the plan
 * and the turn is a slice of it.
 */

import { describe, expect, it } from "vitest";

import { distanceM, metresPerDegreeLon, type LatLng } from "./board";
import type { TerrainSampler } from "./lineOfSight";
import type { AllowanceTable, TerrainClass } from "./movement";
import {
  advanceAlongRoute,
  bearingRoutePlanner,
  rasterRoutePlanner,
  routeInterrupted,
  type PlannedRoute,
} from "./routePlan";

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
const METRES_PER_DEGREE_LAT = 111_320;

function at(east: number, north: number): LatLng {
  return {
    lat: ORIGIN.lat + north / METRES_PER_DEGREE_LAT,
    lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat),
  };
}

function eastingOf(point: LatLng): number {
  return (point.lng - ORIGIN.lng) * metresPerDegreeLon(ORIGIN.lat);
}

function ground(classify: (east: number, north: number) => TerrainClass): TerrainSampler {
  return {
    groundHeightM: () => 0,
    classify: (point) =>
      classify(eastingOf(point), (point.lat - ORIGIN.lat) * METRES_PER_DEGREE_LAT),
  };
}

const OPEN = ground(() => "open");

/** 2,000 m of open ground a turn for tracks; wheels cannot touch marsh. */
const TABLE: AllowanceTable = {
  F: { open: 1000, marsh: 100 },
  W: { open: 2500 },
  T: { open: 2000, marsh: 400 },
};

describe("marching a route over several turns", () => {
  it("spends one turn's allowance and keeps the rest of the route", () => {
    // 6 km of route, 2 km a turn: three turns, and the element must be in the
    // same place at the end as if it had been ordered there each turn.
    const waypoints = [at(2000, 0), at(4000, 0), at(6000, 0)];
    let position = at(0, 0);
    let remaining: LatLng[] = waypoints;
    const reached: number[] = [];

    for (let turn = 0; turn < 3; turn += 1) {
      const progress = advanceAlongRoute(OPEN, "T", TABLE, position, remaining);
      position = progress.end;
      remaining = progress.remaining;
      reached.push(Math.round(eastingOf(position)));
    }

    expect(reached[0]).toBeGreaterThan(1900);
    expect(reached[0]).toBeLessThan(2100);
    expect(reached[2]).toBeGreaterThan(5800);
    expect(remaining).toHaveLength(0);
  });

  it("stops mid-leg rather than rounding to a waypoint", () => {
    // The waypoint is 5 km away and the turn buys 2 km. An element that jumped
    // to the waypoint would teleport; one that refused to move would stall.
    const progress = advanceAlongRoute(OPEN, "T", TABLE, at(0, 0), [at(5000, 0)]);
    expect(progress.distanceM).toBeGreaterThan(1900);
    expect(progress.distanceM).toBeLessThan(2100);
    expect(progress.remaining).toHaveLength(1);
    expect(progress.outcome).toBe("exhausted");
  });

  it("charges the terrain each leg actually crosses", () => {
    // Half the route is marsh, which costs a track five times as much per
    // metre. The turn therefore buys much less than it would in the open.
    const bog = ground((east) => (east > 400 ? "marsh" : "open"));
    const progress = advanceAlongRoute(bog, "T", TABLE, at(0, 0), [at(4000, 0)]);
    expect(progress.distanceM).toBeLessThan(1000);
    expect(progress.distanceM).toBeGreaterThan(400);
  });

  it("has nothing to say about an empty route", () => {
    const progress = advanceAlongRoute(OPEN, "T", TABLE, at(0, 0), []);
    expect(progress.distanceM).toBe(0);
    expect(progress.outcome).toBe("completed");
  });
});

describe("planning a route", () => {
  it("routes round something a bound would walk into", () => {
    // A bog across the axis. The straight line is blocked for wheels, so the
    // plan has to leave the axis — which is exactly what a one-turn bound
    // cannot do, because leaving the axis costs ground that turn.
    const bog = ground((east, north) =>
      east >= 1000 && east <= 2000 && Math.abs(north) <= 1200 ? "marsh" : "open",
    );
    const planner = bearingRoutePlanner(bog, TABLE);
    const waypoints = planner.plan(at(0, 0), at(4000, 0), "W");

    expect(waypoints).not.toBeNull();
    expect(waypoints!.length).toBeGreaterThan(1);
    for (const point of waypoints!) {
      expect(bog.classify(point)).not.toBe("marsh");
    }
  });

  it("says which planner drew it, because they are not equally good", () => {
    expect(bearingRoutePlanner(OPEN, TABLE).kind).toBe("bearings");
  });

  it("gives the same route every time, so a replay is exact", () => {
    const planner = bearingRoutePlanner(OPEN, TABLE);
    const first = planner.plan(at(0, 0), at(5000, 500), "T");
    const second = planner.plan(at(0, 0), at(5000, 500), "T");
    expect(second).toEqual(first);
  });
});

describe("the raster planner", () => {
  const router = {
    route: () => ({
      waypoints: [
        [ORIGIN.lat, ORIGIN.lng],
        [at(1000, 500).lat, at(1000, 500).lng],
        [at(2000, 0).lat, at(2000, 0).lng],
      ] as [number, number][],
    }),
  };

  it("turns the router's polyline into waypoints, dropping the start", () => {
    const planner = rasterRoutePlanner(router, () => "tracked");
    const waypoints = planner.plan(at(0, 0), at(2000, 0), "T");
    expect(planner.kind).toBe("raster");
    expect(waypoints).toHaveLength(2);
    expect(eastingOf(waypoints![1])).toBeCloseTo(2000, -1);
  });

  it("corrects a route handed back end-first", () => {
    // A* reconstructs from the goal backwards. If the reversal upstream ever
    // regressed, the element would march away from its objective — which looks
    // like bad pathfinding rather than a bug for a surprisingly long time.
    const backwards = {
      route: () => ({
        waypoints: [
          [at(2000, 0).lat, at(2000, 0).lng],
          [at(1000, 0).lat, at(1000, 0).lng],
        ] as [number, number][],
      }),
    };
    const planner = rasterRoutePlanner(backwards, () => "tracked");
    const waypoints = planner.plan(at(0, 0), at(3000, 0), "T");
    expect(eastingOf(waypoints![waypoints!.length - 1])).toBeCloseTo(2000, -1);
  });

  it("snaps a goal that sits on impassable ground", () => {
    // AN OBJECTIVE ON A RIVER IS A REAL CASE. A* refuses to plan a route that
    // ends on impassable ground, and the first game played on the real raster
    // had red's objective on one: red never marched, and the only symptom was
    // half the force having no line on the map.
    const asked: [number, number][] = [];
    const picky = {
      isPassable: (lat: number, lng: number) => distanceM({ lat, lng }, at(5000, 0)) > 300,
      route: (fromLat: number, fromLon: number, toLat: number, toLon: number) => {
        asked.push([toLat, toLon]);
        // Refuse anything that ends on the "river", as the real A* does.
        if (distanceM({ lat: toLat, lng: toLon }, at(5000, 0)) <= 300) return null;
        return {
          waypoints: [
            [fromLat, fromLon],
            [toLat, toLon],
          ] as [number, number][],
        };
      },
    };

    const planner = rasterRoutePlanner(picky, () => "tracked");
    const waypoints = planner.plan(at(0, 0), at(5000, 0), "T");

    expect(waypoints).not.toBeNull();
    // It routed somewhere near the objective rather than refusing outright.
    const end = waypoints![waypoints!.length - 1];
    expect(distanceM(end, at(5000, 0))).toBeLessThan(1200);
    expect(distanceM(end, at(5000, 0))).toBeGreaterThan(0);
    expect(asked.length).toBeGreaterThan(0);
  });

  it("reports no route rather than inventing one", () => {
    const planner = rasterRoutePlanner({ route: () => null }, () => "tracked");
    expect(planner.plan(at(0, 0), at(2000, 0), "T")).toBeNull();
  });
});

describe("when a march stops being the plan", () => {
  const route: PlannedRoute = {
    goal: at(5000, 0),
    label: "the objective",
    waypoints: [at(2000, 0), at(5000, 0)],
    plannedOnTurn: 2,
    planner: "bearings",
  };

  it("is interrupted by sighting an enemy", () => {
    // A march is a plan made in the absence of the enemy.
    expect(
      routeInterrupted(route, { sightedEnemy: true, position: at(0, 0), arrivedWithinM: 250 }),
    ).toBe(true);
  });

  it("is finished on arrival", () => {
    expect(
      routeInterrupted(route, { sightedEnemy: false, position: at(4900, 0), arrivedWithinM: 250 }),
    ).toBe(true);
  });

  it("is finished when the waypoints run out", () => {
    expect(
      routeInterrupted(
        { ...route, waypoints: [] },
        { sightedEnemy: false, position: at(0, 0), arrivedWithinM: 250 },
      ),
    ).toBe(true);
  });

  it("otherwise continues", () => {
    expect(
      routeInterrupted(route, { sightedEnemy: false, position: at(1000, 0), arrivedWithinM: 250 }),
    ).toBe(false);
    expect(
      routeInterrupted(undefined, { sightedEnemy: true, position: at(0, 0), arrivedWithinM: 250 }),
    ).toBe(false);
  });

  it("measures arrival from where the element is, not where it started", () => {
    const far = routeInterrupted(route, {
      sightedEnemy: false,
      position: at(0, 0),
      arrivedWithinM: 250,
    });
    const near = routeInterrupted(route, {
      sightedEnemy: false,
      position: at(5000, 100),
      arrivedWithinM: 250,
    });
    expect(far).toBe(false);
    expect(near).toBe(true);
    expect(distanceM(at(5000, 100), route.goal)).toBeLessThan(250);
  });
});
