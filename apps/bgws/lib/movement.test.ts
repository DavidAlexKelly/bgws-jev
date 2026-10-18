import { describe, expect, it } from "vitest";

import { consumeAllowance, isRoadOnly, type AllowanceTable, type RouteLeg } from "./movement";

// The rulebook's own worked example supplies these two numbers for a Foot FE:
// 2,000 m along roads or 1,000 m across open ground, in a turn. The rest are
// invented FOR THE TEST and must never be copied into the app — the real table
// is Player Aid 2.
const TEST_TABLE: AllowanceTable = {
  F: { road: 2000, open: 1000, crops: 800, woodsLight: 600, woodsThick: 400, urban: 600, marsh: 200 },
  W: { road: 6000, open: 2000, crops: 1200, woodsLight: 0, woodsThick: 0, urban: 2000, marsh: 0 },
  T: { road: 5000, open: 3000, crops: 2000, woodsLight: 1200, woodsThick: 600, urban: 1500, marsh: 400 },
};

const START = { lat: 54.71, lng: 20.51 };

/** Legs at a fixed bearing; `to` only has to be self-consistent for these tests. */
function leg(terrain: RouteLeg["terrain"], distanceM: number, index: number): RouteLeg {
  return { terrain, distanceM, to: { lat: START.lat + index * 0.01, lng: START.lng } };
}

describe("maximum allowable distance", () => {
  it("reproduces the rulebook's worked example exactly", () => {
    // 1,000 m of road is half a Foot FE's road allowance; 500 m of open is
    // half its open allowance; together they exhaust the turn precisely.
    const result = consumeAllowance(
      "F",
      [leg("road", 1000, 1), leg("open", 500, 2)],
      TEST_TABLE,
      START,
    );
    expect(result.outcome).toBe("completed");
    expect(result.allowanceSpent).toBeCloseTo(1, 5);
    expect(result.distanceM).toBe(1500);
  });

  it("spends fractions, not metres — mixed terrain costs more than its length", () => {
    // 900 m of thick woods costs a Foot FE more than 900 m of road, and the
    // accumulator has to notice.
    const road = consumeAllowance("F", [leg("road", 900, 1)], TEST_TABLE, START);
    const woods = consumeAllowance("F", [leg("woodsThick", 900, 1)], TEST_TABLE, START);
    expect(road.allowanceSpent).toBeLessThan(woods.allowanceSpent);
    expect(woods.allowanceSpent).toBeGreaterThan(1 - 1e-9);
  });

  it("cuts the last leg where the allowance runs out", () => {
    const result = consumeAllowance(
      "F",
      [leg("road", 1000, 1), leg("open", 900, 2)],
      TEST_TABLE,
      START,
    );
    expect(result.outcome).toBe("exhausted");
    // Half the road allowance is gone, so 500 m of open ground remains.
    expect(result.distanceM).toBeCloseTo(1500, 5);
    expect(result.legs).toHaveLength(2);
    expect(result.legs[1].distanceM).toBeCloseTo(500, 5);
  });

  it("stops at impassable ground rather than swimming across it", () => {
    // A wheeled FE cannot cross marsh at any speed. Impassable is not slow.
    const result = consumeAllowance(
      "W",
      [leg("road", 500, 1), leg("marsh", 100, 2)],
      TEST_TABLE,
      START,
    );
    expect(result.outcome).toBe("blocked");
    expect(result.blockedBy).toBe("marsh");
    expect(result.distanceM).toBe(500);
  });

  it("treats a terrain the table does not mention as impassable", () => {
    // Silence is not permission: an unlisted terrain is one the scenario
    // author has not priced, and guessing would invent a rule.
    const result = consumeAllowance("W", [leg("water", 50, 1)], TEST_TABLE, START);
    expect(result.outcome).toBe("blocked");
  });

  it("ends where it stopped, not where it was going", () => {
    const result = consumeAllowance(
      "F",
      [leg("road", 4000, 1)],
      TEST_TABLE,
      START,
    );
    expect(result.outcome).toBe("exhausted");
    expect(result.distanceM).toBeCloseTo(2000, 5);
    // Half of the leg, so half of the way to its end point.
    expect(result.end.lat).toBeCloseTo(START.lat + 0.005, 6);
  });
});

describe("column movement", () => {
  it("needs road for the whole move", () => {
    expect(isRoadOnly([leg("road", 100, 1), leg("road", 100, 2)])).toBe(true);
    expect(isRoadOnly([leg("road", 100, 1), leg("open", 10, 2)])).toBe(false);
    expect(isRoadOnly([])).toBe(false);
  });
});
