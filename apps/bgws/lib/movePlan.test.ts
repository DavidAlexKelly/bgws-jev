import { describe, expect, it } from "vitest";

import { distanceM, metresPerDegreeLon, type LatLng } from "./board";
import type { TerrainSampler } from "./lineOfSight";
import type { AllowanceTable, TerrainClass } from "./movement";
import { describeMovePlan, legsAlong, planMove, planMoveAround } from "./movePlan";

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
const METRES_PER_DEGREE_LAT = 111_320;

/** A point `east`/`north` metres from the origin. */
function at(east: number, north: number): LatLng {
  return {
    lat: ORIGIN.lat + north / METRES_PER_DEGREE_LAT,
    lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat),
  };
}

function eastingOf(point: LatLng): number {
  return (point.lng - ORIGIN.lng) * metresPerDegreeLon(ORIGIN.lat);
}

function northingOf(point: LatLng): number {
  return (point.lat - ORIGIN.lat) * METRES_PER_DEGREE_LAT;
}

/** Terrain decided by a function of the metre grid. Flat, so only class matters. */
function groundWhere(classify: (east: number, north: number) => TerrainClass): TerrainSampler {
  return {
    groundHeightM: () => 0,
    classify: (point) => classify(eastingOf(point), northingOf(point)),
  };
}

const OPEN = groundWhere(() => "open");

/**
 * Numbers OURS, and deliberately round: these test the mechanism, not a table.
 * Tracks cross marsh slowly, wheels not at all — which is the repo's own speed
 * table, and is the distinction the whole rule exists to make.
 */
const TABLE: AllowanceTable = {
  F: { open: 1000, road: 2000, woodsLight: 400, marsh: 100 },
  W: { open: 5000, road: 10_000, woodsLight: 600 },
  T: { open: 2000, road: 4000, woodsLight: 800, marsh: 400 },
};

describe("legsAlong", () => {
  it("samples the line and classifies each piece", () => {
    const legs = legsAlong(OPEN, at(0, 0), at(1000, 0), 100);
    expect(legs).toHaveLength(10);
    expect(legs.every((leg) => leg.terrain === "open")).toBe(true);
    expect(legs.reduce((sum, leg) => sum + leg.distanceM, 0)).toBeCloseTo(1000, -1);
    expect(eastingOf(legs[legs.length - 1].to)).toBeCloseTo(1000, -1);
  });

  it("classifies a leg by its midpoint, not by where it starts", () => {
    // Boundary at 150 m: a 100 m leg starting at 100 m is mostly marsh.
    const ground = groundWhere((east) => (east >= 150 ? "marsh" : "open"));
    const legs = legsAlong(ground, at(0, 0), at(300, 0), 100);
    expect(legs.map((leg) => leg.terrain)).toEqual(["open", "marsh", "marsh"]);
  });

  it("has nothing to say about going nowhere", () => {
    expect(legsAlong(OPEN, at(0, 0), at(0, 0), 100)).toEqual([]);
  });
});

describe("planMove", () => {
  it("arrives when the going allows it", () => {
    const plan = planMove({
      terrain: OPEN,
      moveType: "T",
      table: TABLE,
      from: at(0, 0),
      to: at(800, 0),
    });
    expect(plan.outcome).toBe("completed");
    expect(plan.distanceM).toBeCloseTo(800, -1);
    expect(eastingOf(plan.destination)).toBeCloseTo(800, -1);
  });

  it("stops where the turn runs out", () => {
    // 2,000 m of open is the whole turn for tracks; 3,000 m was asked for.
    const plan = planMove({
      terrain: OPEN,
      moveType: "T",
      table: TABLE,
      from: at(0, 0),
      to: at(3000, 0),
    });
    expect(plan.outcome).toBe("exhausted");
    expect(plan.distanceM).toBeCloseTo(2000, -1);
    expect(plan.allowanceSpent).toBeCloseTo(1, 2);
  });

  it("charges mixed terrain in fractions of the turn, not in metres", () => {
    // 400 m of light woods is half a tracked turn (800 m allowance); 1,000 m
    // of open is the other half (2,000 m allowance). Together: exactly one.
    const ground = groundWhere((east) => (east < 400 ? "woodsLight" : "open"));
    const plan = planMove({
      terrain: ground,
      moveType: "T",
      table: TABLE,
      from: at(0, 0),
      to: at(2000, 0),
    });
    expect(plan.distanceM).toBeCloseTo(1400, -2);
  });

  it("will not enter ground the Move Type cannot cross", () => {
    const ground = groundWhere((east) => (east >= 500 ? "marsh" : "open"));
    const plan = planMove({
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to: at(1500, 0),
    });
    expect(plan.outcome).toBe("blocked");
    expect(plan.blockedBy).toBe("marsh");
    // Stopped at the edge, not part way through it.
    expect(eastingOf(plan.destination)).toBeLessThanOrEqual(550);
  });

  it("lets tracks cross what stops wheels", () => {
    const ground = groundWhere((east) => (east >= 500 ? "marsh" : "open"));
    const request = { terrain: ground, table: TABLE, from: at(0, 0), to: at(800, 0) } as const;
    expect(planMove({ ...request, moveType: "W" }).outcome).toBe("blocked");
    expect(planMove({ ...request, moveType: "T" }).outcome).toBe("completed");
  });

  it("never traps an element in the ground it is standing on", () => {
    // A truck generated inside a marsh. It pays the worst rate it has, and it
    // gets out — an immobile element no rule put there would be found as "the
    // movement check is broken".
    const ground = groundWhere((east) => (east < 300 ? "marsh" : "open"));
    const plan = planMove({
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to: at(900, 0),
    });
    expect(plan.distanceM).toBeGreaterThan(0);
    expect(plan.outcome).not.toBe("blocked");
  });

  it("does not let that become a licence to re-enter it", () => {
    const ground = groundWhere((east) => (east < 200 || east >= 600 ? "marsh" : "open"));
    const plan = planMove({
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to: at(1200, 0),
    });
    expect(plan.outcome).toBe("blocked");
    expect(eastingOf(plan.destination)).toBeLessThan(700);
  });

  it("never ends the move inside ground the Move Type cannot enter", () => {
    // The sample grid does not line up with the edge of a bog. A leg priced by
    // its midpoint can be legal by the metre and still deliver the element
    // inside the marsh — which it did, until the end point was walked back.
    const ground = groundWhere((east) => (east >= 549 ? "marsh" : "open"));
    for (let ask = 600; ask <= 2000; ask += 37) {
      const plan = planMove({
        terrain: ground,
        moveType: "W",
        table: TABLE,
        from: at(0, 0),
        to: at(ask, 0),
      });
      expect(ground.classify(plan.destination), `ask ${ask} m`).not.toBe("marsh");
      expect(plan.outcome, `ask ${ask} m`).toBe("blocked");
    }
  });

  it("reports being stopped short as blocked, not as arriving", () => {
    // Trimming the end back is not a cosmetic adjustment: an element that
    // stopped at a bog was stopped BY the bog, and the summary a commander
    // reads has to say that rather than "arrived".
    const ground = groundWhere((east) => (east >= 549 ? "marsh" : "open"));
    const plan = planMove({
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to: at(700, 0),
    });
    expect(plan.outcome).toBe("blocked");
    expect(plan.blockedBy).toBe("marsh");
    expect(plan.distanceM).toBeLessThan(560);
    expect(plan.distanceM).toBeGreaterThan(400);
  });

  it("gives the same answer every time", () => {
    const request = {
      terrain: groundWhere((east) => (east > 300 ? "woodsLight" : "open")),
      moveType: "F",
      table: TABLE,
      from: at(0, 0),
      to: at(900, 100),
    } as const;
    const first = planMove(request);
    const second = planMove(request);
    expect(second).toEqual(first);
  });
});

describe("planMoveAround", () => {
  it("goes straight when straight works, and says nothing about it", () => {
    const plan = planMoveAround({
      terrain: OPEN,
      moveType: "T",
      table: TABLE,
      from: at(0, 0),
      to: at(900, 0),
    });
    expect(plan.outcome).toBe("completed");
    expect(plan.detouredDeg).toBeUndefined();
  });

  it("goes round a patch it cannot cross", () => {
    // A bog straddling the axis between 400 m and 900 m east, 400 m wide.
    const ground = groundWhere((east, north) =>
      east >= 400 && east <= 900 && Math.abs(north) <= 200 ? "marsh" : "open",
    );
    const from = at(0, 0);
    const to = at(1200, 0);
    const direct = planMove({ terrain: ground, moveType: "W", table: TABLE, from, to });
    const around = planMoveAround({ terrain: ground, moveType: "W", table: TABLE, from, to });

    expect(direct.outcome).toBe("blocked");
    expect(around.detouredDeg).toBeDefined();
    expect(around.detouredAround).toBe("marsh");
    // The test that matters: it ends up closer to where it wanted to be.
    expect(distanceM(around.destination, to)).toBeLessThan(distanceM(direct.destination, to));
  });

  it("prefers progress towards the objective over distance covered", () => {
    const ground = groundWhere((east, north) =>
      east >= 300 && Math.abs(north) <= 150 ? "marsh" : "open",
    );
    const to = at(1500, 0);
    const plan = planMoveAround({
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to,
    });
    expect(eastingOf(plan.destination)).toBeGreaterThan(300);
  });

  it("breaks ties the same way every time, so a replay is exact", () => {
    // Symmetric obstacle: left and right are equally good, and the rule is
    // that the first bearing to beat the incumbent wins.
    const ground = groundWhere((east, north) =>
      east >= 400 && east <= 800 && Math.abs(north) <= 200 ? "marsh" : "open",
    );
    const request = {
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to: at(1200, 0),
    } as const;
    const first = planMoveAround(request);
    for (let i = 0; i < 5; i += 1) {
      expect(planMoveAround(request)).toEqual(first);
    }
  });
});

describe("describeMovePlan", () => {
  it("says how far, and why not further", () => {
    const exhausted = planMove({
      terrain: OPEN,
      moveType: "T",
      table: TABLE,
      from: at(0, 0),
      to: at(5000, 0),
    });
    expect(describeMovePlan(exhausted)).toContain("as far as one turn allows");

    const ground = groundWhere((east) => (east >= 400 ? "marsh" : "open"));
    const blocked = planMove({
      terrain: ground,
      moveType: "W",
      table: TABLE,
      from: at(0, 0),
      to: at(1500, 0),
    });
    expect(describeMovePlan(blocked)).toContain("halted by marsh");
  });
});
