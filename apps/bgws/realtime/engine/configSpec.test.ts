// configSpec: a config built from data plays the same game as one built by
// hand the way RealtimePlay used to, and a raster rebuilt from its exported
// grid (as a worker would get it) plays the same game too.

import { createHash } from "node:crypto";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { TerrainRaster } from "../../../../shared/routing/terrainRaster";
import type { LatLng } from "../../lib/board";
import { DEFAULT_ORIGIN, scenarioFactory } from "../../lib/forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "../../lib/proceduralTerrain";
import { rasterTerrain } from "../../lib/rasterTerrain";
import { rasterRoutePlanner } from "../../lib/routePlan";
import { mobilityClassFor } from "../../lib/terrainAdapter";
import { createRng } from "../../rules/dice";
import { SYMMETRIC_CONTROL_V1 } from "../../rules/forceList";
import { HOUSE_V1 } from "../../rules/ruleset";
import { configFromSpec, RT_CONFIG_SPEC_VERSION, rulesetById, timingSpec, type RtConfigSpec } from "./configSpec";
import { ruleDecider } from "./deciders";
import { createRealtimeState } from "./engine";
import { heuristicInitialOrders } from "./orders";
import { RealtimeRunner } from "./runner";
import { loadedRaster } from "./__fixtures__/syntheticRaster";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig } from "./types";

// TerrainRaster logs its loading and every route; keep the test output readable.
beforeAll(() => {
  vi.spyOn(console, "log").mockImplementation(() => {});
});
afterAll(() => {
  vi.restoreAllMocks();
});

const TIMING = { ...DEFAULT_TIMING, breakpoint: 0.5, maxDurationS: 1800 };

/** Final state of a seeded game, as a hash. */
async function playHash(config: RtConfig, seconds?: number): Promise<string> {
  let state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
  state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", config), "red", config);
  const runner = new RealtimeRunner(state, config, { deciders: { blue: ruleDecider, red: ruleDecider } });
  if (seconds != null) await runner.advance(seconds);
  else while (!runner.state.over) await runner.advance(300);
  return createHash("sha256").update(JSON.stringify(runner.state)).digest("hex");
}

function spec(ground: RtConfigSpec["ground"], seed = "spec-test"): RtConfigSpec {
  return { version: RT_CONFIG_SPEC_VERSION, rulesetId: HOUSE_V1.id, seed, timing: timingSpec(TIMING), comms: "radio", ground };
}

/** RealtimePlay's config before this change, for raster ground. */
function handBuiltRasterConfig(raster: TerrainRaster, relief: boolean, seed: string, origin: LatLng): RtConfig {
  return {
    ruleset: HOUSE_V1,
    terrain: rasterTerrain(raster, { relief: relief ? { ...STANDARD_GROUND, seed: "ground-1", origin } : null }),
    rng: createRng(seed),
    timing: TIMING,
    planner: rasterRoutePlanner(raster, mobilityClassFor),
    isPassable: (point) => raster.isPassable(point.lat, point.lng),
    comms: "radio",
  };
}

describe("RtConfigSpec", () => {
  it("is plain data: survives JSON and structured clone unchanged", () => {
    const s: RtConfigSpec = {
      ...spec({ source: "raster+relief", seed: "g", origin: DEFAULT_ORIGIN, rasterRid: "ri.test.raster" }),
      rngCursor: 12,
      behaviours: { B1: "core/armour" },
      behaviourTemplates: [],
    };
    expect(JSON.parse(JSON.stringify(s))).toEqual(s);
    expect(structuredClone(s)).toEqual(s);
  });

  it("generated ground: the same game as the hand-built config, seed for seed", async () => {
    for (const seed of ["spec0", "spec1", "spec2"]) {
      const hand: RtConfig = {
        ruleset: HOUSE_V1,
        terrain: proceduralTerrain({ ...STANDARD_GROUND, seed: "ground-1", origin: DEFAULT_ORIGIN }),
        rng: createRng(seed),
        timing: TIMING,
        comms: "radio",
      };
      const fromSpec = configFromSpec(spec({ source: "generated", seed: "ground-1", origin: DEFAULT_ORIGIN }, seed));
      expect(fromSpec.planner).toBeUndefined();
      expect(fromSpec.isPassable).toBeUndefined();
      expect(await playHash(fromSpec)).toBe(await playHash(hand));
    }
  }, 60_000);

  it("raster ground: the same game as the hand-built config, and from a re-imported grid", async () => {
    const raster = await loadedRaster();
    const copy = TerrainRaster.fromGrid(raster.exportGrid()!);
    const hashes: string[] = [];
    for (const source of ["raster", "raster+relief"] as const) {
      const ground = { source, seed: "ground-1", origin: DEFAULT_ORIGIN, rasterRid: "ri.test.raster" };
      const hand = await playHash(handBuiltRasterConfig(raster, source === "raster+relief", "spec-raster", DEFAULT_ORIGIN), 600);
      expect(await playHash(configFromSpec(spec(ground, "spec-raster"), { raster }), 600)).toBe(hand);
      expect(await playHash(configFromSpec(spec(ground, "spec-raster"), { raster: copy }), 600)).toBe(hand);
      hashes.push(hand);
    }
    // The relief is really applied: flat and hilly ground play differently.
    expect(hashes[0]).not.toBe(hashes[1]);
  }, 120_000);

  it("timing round-trips, with the standard reaction time put back", () => {
    const timing = configFromSpec(spec({ source: "generated", seed: "g", origin: DEFAULT_ORIGIN })).timing;
    expect(timing).toEqual(TIMING);
    expect(timing.reactionS).toBe(DEFAULT_TIMING.reactionS);
    expect(() => timingSpec({ ...TIMING, reactionS: () => 1 })).toThrow(/reactionS/);
  });

  it("resumes the dice from rngCursor", () => {
    const a = configFromSpec({ ...spec({ source: "generated", seed: "g", origin: DEFAULT_ORIGIN }), rngCursor: 5 }).rng;
    const b = createRng("spec-test");
    for (let i = 0; i < 5; i++) b.int(1_000_000);
    expect(a.int(1_000_000)).toBe(b.int(1_000_000));
  });

  it("refuses what it cannot build faithfully", () => {
    expect(() => rulesetById("nope")).toThrow(/Unknown ruleset/);
    expect(() => configFromSpec({ ...spec({ source: "generated", seed: "g", origin: DEFAULT_ORIGIN }), rulesetId: "nope" })).toThrow(/Unknown ruleset/);
    expect(() => configFromSpec(spec({ source: "raster", seed: "g", origin: DEFAULT_ORIGIN, rasterRid: "ri.x" }))).toThrow(/needs the terrain raster/);
    expect(() => configFromSpec({ ...spec({ source: "generated", seed: "g", origin: DEFAULT_ORIGIN }), version: 2 as 1 })).toThrow(/version/);
  });
});

describe("TerrainRaster grid export", () => {
  it("a raster rebuilt from its exported grid answers exactly as the loaded one", async () => {
    const raster = await loadedRaster();
    const grid = raster.exportGrid()!;
    // As a worker would receive it: the bytes transferred, not shared.
    const copy = TerrainRaster.fromGrid(structuredClone(grid));
    expect(copy.isLoaded).toBe(true);
    expect(copy.bounds).toEqual(raster.bounds);
    for (let lat = 53.99; lat <= 54.41; lat += 0.0037) {
      for (let lng = 18.29; lng <= 18.91; lng += 0.0041) {
        expect(copy.flagAt(lat, lng)).toBe(raster.flagAt(lat, lng));
        expect(copy.isPassable(lat, lng)).toBe(raster.isPassable(lat, lng));
      }
    }
    const route = (r: TerrainRaster) => r.route(54.15, 18.5, 54.25, 18.75, "tracked");
    expect(route(copy)).toEqual(route(raster));
    expect(route(raster)).not.toBeNull();
    // Exported data is a copy: changing it does not change the raster.
    grid.data.fill(255);
    expect(raster.isPassable(54.2, 18.6)).toBe(true);
  });

  it("refuses a grid whose bytes do not match its size", () => {
    expect(() =>
      TerrainRaster.fromGrid({ minLat: 0, minLon: 0, cellSize: 1, numRows: 2, numCols: 2, data: new Uint8Array(3) }),
    ).toThrow(/size mismatch/);
  });
});
