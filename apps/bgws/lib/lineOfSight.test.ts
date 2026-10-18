import { describe, expect, it } from "vitest";

import { distanceM, type LatLng } from "./board";
import {
  FEATURE_HEIGHT_M,
  flatTerrain,
  lineOfSight,
  type TerrainSampler,
} from "./lineOfSight";
import type { TerrainClass } from "./movement";

const WEST = { lat: 54.71, lng: 20.51 };

/** A point `metres` east of WEST. */
function east(metres: number): LatLng {
  const mPerDegLon = 111_320 * Math.cos((WEST.lat * Math.PI) / 180);
  return { lat: WEST.lat, lng: WEST.lng + metres / mPerDegLon };
}

/**
 * Terrain that is flat except for a band between two eastings, where it is
 * either raised ground or a blocking feature.
 */
function terrainWithBand(options: {
  fromM: number;
  toM: number;
  heightM?: number;
  klass?: TerrainClass;
}): TerrainSampler {
  const mPerDegLon = 111_320 * Math.cos((WEST.lat * Math.PI) / 180);
  const eastingOf = (p: LatLng) => (p.lng - WEST.lng) * mPerDegLon;
  const inBand = (p: LatLng) => {
    const e = eastingOf(p);
    return e >= options.fromM && e <= options.toM;
  };
  return {
    groundHeightM: (p) => (inBand(p) ? (options.heightM ?? 0) : 0),
    classify: (p) => (inBand(p) ? (options.klass ?? "open") : "open"),
  };
}

describe("the 3 km cap", () => {
  it("blocks anything beyond it, however flat the ground", () => {
    const result = lineOfSight(flatTerrain(), { from: WEST, to: east(3200) });
    expect(result.visible).toBe(false);
    expect(result.reason).toBe("beyondRange");
  });

  it("allows the same line just inside it", () => {
    const result = lineOfSight(flatTerrain(), { from: WEST, to: east(2900) });
    expect(result.visible).toBe(true);
    expect(result.rangeM).toBeGreaterThan(2800);
  });
});

describe("ground", () => {
  it("sees across flat, bare ground", () => {
    expect(lineOfSight(flatTerrain(120), { from: WEST, to: east(2000) }).visible).toBe(true);
  });

  it("is blocked by a hill between the two", () => {
    const terrain = terrainWithBand({ fromM: 900, toM: 1100, heightM: 60 });
    const result = lineOfSight(terrain, { from: WEST, to: east(2000) });
    expect(result.visible).toBe(false);
    expect(result.reason).toBe("ground");
    expect(result.obstructionM).toBeGreaterThan(50);
    expect(distanceM(WEST, result.blockedAt!)).toBeGreaterThan(800);
  });

  it("sees over the same hill from higher ground", () => {
    // The observer on a 200 m feature looks down on a 60 m rise.
    const terrain: TerrainSampler = {
      groundHeightM: (p) => (p.lng === WEST.lng ? 200 : 0),
      classify: () => "open",
    };
    const raised = terrainWithBand({ fromM: 900, toM: 1100, heightM: 60 });
    const combined: TerrainSampler = {
      groundHeightM: (p) =>
        p.lng === WEST.lng ? terrain.groundHeightM(p) : raised.groundHeightM(p),
      classify: raised.classify,
    };
    expect(lineOfSight(combined, { from: WEST, to: east(2000) }).visible).toBe(true);
  });
});

describe("features on a bare-earth DEM", () => {
  it("is blocked by woods, which the DEM does not contain", () => {
    // Ground is flat: without the +20 m correction this line reads clear, and
    // a wood would stop concealing anything for the whole game.
    const terrain = terrainWithBand({ fromM: 800, toM: 1200, klass: "woodsThick" });
    const result = lineOfSight(terrain, { from: WEST, to: east(2000) });
    expect(result.visible).toBe(false);
    expect(result.reason).toBe("feature");
  });

  it("is blocked by urban ground for the same reason", () => {
    const terrain = terrainWithBand({ fromM: 500, toM: 700, klass: "urban" });
    expect(lineOfSight(terrain, { from: WEST, to: east(1500) }).visible).toBe(false);
  });

  it("sees over a wood from far enough above it", () => {
    // Observer 40 m up, target 40 m up, wood 20 m tall on flat ground between.
    const terrain = terrainWithBand({ fromM: 800, toM: 1200, klass: "woodsLight" });
    const result = lineOfSight(terrain, {
      from: WEST,
      to: east(2000),
      observerHeightM: FEATURE_HEIGHT_M + 20,
      targetHeightM: FEATURE_HEIGHT_M + 20,
    });
    expect(result.visible).toBe(true);
  });
});

describe("counters", () => {
  it("blocks when one stands on the line at the same elevation", () => {
    const result = lineOfSight(flatTerrain(), {
      from: WEST,
      to: east(2000),
      counters: [{ position: east(1000) }],
    });
    expect(result.visible).toBe(false);
    expect(result.reason).toBe("counter");
  });

  it("ignores one standing well off the line", () => {
    const offLine = { lat: WEST.lat + 0.01, lng: east(1000).lng };
    const result = lineOfSight(flatTerrain(), {
      from: WEST,
      to: east(2000),
      counters: [{ position: offLine }],
    });
    expect(result.visible).toBe(true);
  });
});

describe("coverage", () => {
  it("reports missing DEM rather than claiming a clear view", () => {
    const noData: TerrainSampler = {
      groundHeightM: () => Number.NaN,
      classify: () => "open",
    };
    const result = lineOfSight(noData, { from: WEST, to: east(1000) });
    expect(result.visible).toBe(false);
    expect(result.reason).toBe("noCoverage");
  });
});
