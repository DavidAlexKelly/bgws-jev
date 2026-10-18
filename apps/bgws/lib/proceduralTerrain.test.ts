import { describe, expect, it } from "vitest";

import { lineOfSight } from "./lineOfSight";
import {
  inCover,
  proceduralTerrain,
  STANDARD_GROUND,
  STANDARD_GROUND_V1,
  STANDARD_GROUND_V2,
} from "./proceduralTerrain";

const ORIGIN = { lat: 54.2, lng: 18.6 };

/** A point a given number of metres from the origin. */
function at(eastM: number, northM: number) {
  const perLng = 111_320 * Math.cos((ORIGIN.lat * Math.PI) / 180);
  return { lat: ORIGIN.lat + northM / 111_320, lng: ORIGIN.lng + eastM / perLng };
}

describe("determinism", () => {
  it("gives the same ground for the same seed", () => {
    // A batch result is only reproducible if the ground is. If this fails,
    // every recorded finding becomes unrepeatable.
    const a = proceduralTerrain({ seed: "x" });
    const b = proceduralTerrain({ seed: "x" });
    for (const [east, north] of [[0, 0], [500, 300], [-1200, 2400]]) {
      expect(a.groundHeightM(at(east, north))).toBe(b.groundHeightM(at(east, north)));
      expect(a.classify(at(east, north))).toBe(b.classify(at(east, north)));
    }
  });

  it("gives different ground for a different seed", () => {
    const a = proceduralTerrain({ seed: "x" });
    const b = proceduralTerrain({ seed: "y" });
    const heights = [0, 400, 800, 1200, 1600].map(
      (n) => a.groundHeightM(at(0, n)) - b.groundHeightM(at(0, n)),
    );
    expect(heights.some((d) => Math.abs(d) > 1)).toBe(true);
  });
});

describe("elevation", () => {
  it("is not flat", () => {
    // The whole reason this module exists: flat ground makes every cover and
    // elevation rule unreachable, so a sweep reports them as having no effect.
    const terrain = proceduralTerrain({ seed: "relief", reliefM: 60 });
    const heights = Array.from({ length: 40 }, (_, i) => terrain.groundHeightM(at(i * 120, 0)));
    expect(Math.max(...heights) - Math.min(...heights)).toBeGreaterThan(10);
  });

  it("stays within the requested relief", () => {
    const terrain = proceduralTerrain({ seed: "relief", reliefM: 40 });
    for (let i = 0; i < 60; i += 1) {
      const h = terrain.groundHeightM(at(i * 200, i * 130));
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThanOrEqual(40);
    }
  });

  it("varies smoothly rather than in steps", () => {
    // A sight line samples every 50 m; cliff edges between cells would make
    // visibility depend on sample alignment rather than on the ground.
    const terrain = proceduralTerrain({ seed: "smooth", reliefM: 60 });
    let worst = 0;
    for (let i = 0; i < 100; i += 1) {
      const a = terrain.groundHeightM(at(i * 10, 0));
      const b = terrain.groundHeightM(at((i + 1) * 10, 0));
      worst = Math.max(worst, Math.abs(a - b));
    }
    // Under 10 m over 10 m of ground is a 45 degree slope at worst.
    expect(worst).toBeLessThan(10);
  });
});

describe("cover", () => {
  it("produces some woodland and some open ground", () => {
    const terrain = proceduralTerrain({ seed: "cover", woodFraction: 0.3 });
    const classes = new Set<string>();
    for (let i = 0; i < 400; i += 1) classes.add(terrain.classify(at(i * 60, i * 37)));
    expect(classes.has("open")).toBe(true);
    expect([...classes].some((c) => c.startsWith("woods"))).toBe(true);
  });

  it("puts roughly the requested fraction under wood", () => {
    const terrain = proceduralTerrain({ seed: "cover", woodFraction: 0.3 });
    let wooded = 0;
    const total = 2000;
    for (let i = 0; i < total; i += 1) {
      if (inCover(terrain, at((i % 50) * 200, Math.floor(i / 50) * 200))) wooded += 1;
    }
    // Generous band: the point is that the knob has the right sign and rough
    // magnitude, not that value noise hits a quota.
    expect(wooded / total).toBeGreaterThan(0.1);
    expect(wooded / total).toBeLessThan(0.6);
  });

  it("none at all when asked for none", () => {
    const terrain = proceduralTerrain({ seed: "cover", woodFraction: 0 });
    for (let i = 0; i < 200; i += 1) {
      expect(inCover(terrain, at(i * 90, i * 55))).toBe(false);
    }
  });
});

describe("effect on line of sight", () => {
  it("blocks some lines that flat ground would not", () => {
    // If terrain never blocked a sight line it would not be terrain.
    const terrain = proceduralTerrain({ seed: "los", reliefM: 60, woodFraction: 0.3 });
    let blocked = 0;
    for (let i = 0; i < 60; i += 1) {
      const from = at(i * 100, 0);
      const to = at(i * 100, 2000);
      if (!lineOfSight(terrain, { from, to }).visible) blocked += 1;
    }
    expect(blocked).toBeGreaterThan(0);
  });
});

describe("wet ground", () => {
  it("is absent unless asked for, so every older ground is unchanged", () => {
    const dry = proceduralTerrain({ seed: "wet-test", woodFraction: 0.1 });
    const classes = new Set<string>();
    for (let i = 0; i < 400; i += 1) {
      classes.add(dry.classify(at((i % 20) * 300, Math.floor(i / 20) * 300)));
    }
    expect(classes.has("marsh")).toBe(false);
    expect(classes.has("water")).toBe(false);
  });

  it("puts marsh and water on the board when asked", () => {
    const wet = proceduralTerrain({ seed: "wet-test", woodFraction: 0.1, wetFraction: 0.3 });
    const counts: Record<string, number> = {};
    for (let i = 0; i < 2500; i += 1) {
      const klass = wet.classify(at((i % 50) * 200, Math.floor(i / 50) * 200));
      counts[klass] = (counts[klass] ?? 0) + 1;
    }
    expect(counts.marsh ?? 0).toBeGreaterThan(0);
    expect(counts.water ?? 0).toBeGreaterThan(0);
    // Still mostly dry: ground that is half bog is as uninformative as ground
    // that is all open, just in the other direction.
    expect((counts.open ?? 0) / 2500).toBeGreaterThan(0.5);
  });

  it("comes in patches a route can go round, not speckle", () => {
    const wet = proceduralTerrain({ seed: "wet-test", woodFraction: 0, wetFraction: 0.3 });
    // Walk east until wet ground is found, then measure how far it runs.
    let start = -1;
    for (let i = 0; i < 200 && start < 0; i += 1) {
      if (wet.classify(at(i * 50, 0)) !== "open") start = i;
    }
    expect(start).toBeGreaterThanOrEqual(0);
    let run = 0;
    while (wet.classify(at((start + run) * 50, 0)) !== "open") run += 1;
    // 600 m cells: a patch should be hundreds of metres across, not 50.
    expect(run * 50).toBeGreaterThanOrEqual(150);
  });

  it("is the same ground every time, wet included", () => {
    const a = proceduralTerrain({ seed: "repeat", wetFraction: 0.3 });
    const b = proceduralTerrain({ seed: "repeat", wetFraction: 0.3 });
    for (let i = 0; i < 50; i += 1) {
      expect(a.classify(at(i * 137, i * 91))).toBe(b.classify(at(i * 137, i * 91)));
    }
  });
});

describe("the standard ground", () => {
  it("v2 has ground that can actually stop something", () => {
    // The point of v2. v1 is 97% open and every Move Type crosses all of it,
    // so the movement allowance could never bind and the sweep priced it at
    // zero — a rule that cannot be measured because the ground asks nothing.
    const ground = proceduralTerrain(STANDARD_GROUND_V2);
    const counts: Record<string, number> = {};
    for (let i = 0; i < 2500; i += 1) {
      const klass = ground.classify(at((i % 50) * 200, Math.floor(i / 50) * 200));
      counts[klass] = (counts[klass] ?? 0) + 1;
    }
    expect((counts.marsh ?? 0) + (counts.water ?? 0)).toBeGreaterThan(0);
    // And still a battlefield rather than a swamp.
    expect((counts.open ?? 0) / 2500).toBeGreaterThan(0.8);
  });

  it("names a different seed from v1, so a replay identifies one ground", () => {
    expect(STANDARD_GROUND_V2.seed).not.toBe(STANDARD_GROUND_V1.seed);
    expect(STANDARD_GROUND).toBe(STANDARD_GROUND_V2);
  });

  it("is gentler than it looks like it should be, on purpose", () => {
    // At 30% woodland both sides sit in cover and games run to the turn
    // limit, which measures nothing. Swept, not chosen — and if someone
    // raises these, game length is what to check.
    expect(STANDARD_GROUND_V1.woodFraction).toBeLessThanOrEqual(0.15);
    expect(STANDARD_GROUND_V1.reliefM).toBeLessThanOrEqual(30);
  });

  it("is versioned, because it decides outcomes", () => {
    expect(STANDARD_GROUND_V1.seed).toBeTruthy();
  });
});
