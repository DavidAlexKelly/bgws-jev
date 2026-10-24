// The runner in a "worker" — workerHost over a real MessageChannel, so every
// message is structured-cloned exactly as between threads — plays the same
// game as the runner on this thread.

import { createHash } from "node:crypto";
import { MessageChannel } from "node:worker_threads";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { DEFAULT_ORIGIN, scenarioFactory } from "../../lib/forceBuilder";
import { COMBINED_ARMS_V1, SYMMETRIC_CONTROL_V1, type ForceList } from "../../rules/forceList";
import { createRng } from "../../rules/dice";
import { HOUSE_V1 } from "../../rules/ruleset";
import { loadedRaster } from "./__fixtures__/syntheticRaster";
import { configFromSpec, RT_CONFIG_SPEC_VERSION, timingSpec, type RtConfigSpec, type RtRaster } from "./configSpec";
import { ruleDecider, type RtDecider } from "./deciders";
import { createRealtimeState } from "./engine";
import type { EntryContext } from "./feed";
import { heuristicInitialOrders, heuristicOrders } from "./orders";
import { InThreadRunner, WorkerRunner, type HandleOptions, type RunnerHandle } from "./runnerHandle";
import type { RtLogEntry } from "./runner";
import { DEFAULT_TIMING } from "./timing";
import type { RtState } from "./types";
import { hostRunner, type MessageEndpoint } from "./workerHost";
import type { TerrainRaster } from "../../../../shared/routing/terrainRaster";

beforeAll(() => {
  vi.spyOn(console, "log").mockImplementation(() => {});
});
afterAll(() => {
  vi.restoreAllMocks();
});

const hash = (value: unknown) => createHash("sha256").update(JSON.stringify(value)).digest("hex");

function spec(seed: string, ground: RtConfigSpec["ground"] = { source: "generated", seed: "g1", origin: DEFAULT_ORIGIN }): RtConfigSpec {
  return {
    version: RT_CONFIG_SPEC_VERSION,
    rulesetId: HOUSE_V1.id,
    seed,
    timing: timingSpec({ ...DEFAULT_TIMING, maxDurationS: 1800 }),
    comms: "radio",
    ground,
  };
}

function opening(list: ForceList, s: RtConfigSpec, raster?: RtRaster | null): RtState {
  const config = configFromSpec(s, { raster });
  const state = createRealtimeState(scenarioFactory(list, HOUSE_V1)());
  return heuristicInitialOrders(heuristicInitialOrders(state, "blue", config), "red", config);
}

/** A runner in a "worker" on the far side of a MessageChannel. */
function overChannel(s: RtConfigSpec, initial: RtState, options: HandleOptions, raster?: TerrainRaster): WorkerRunner {
  const { port1, port2 } = new MessageChannel();
  const main = port1 as unknown as MessageEndpoint;
  const far = port2 as unknown as MessageEndpoint;
  port1.start();
  port2.start();
  const stopHost = hostRunner(far);
  return new WorkerRunner(
    main,
    { spec: s, raster: raster?.exportGrid() ?? null, initial, config: configFromSpec(s, { raster }) },
    options,
    () => {
      stopHost();
      port1.close();
      port2.close();
    },
  );
}

/** Rules decisions, answered after a varying real delay (0-3 ms): the round trip must not matter. */
function slowRules(calls: string[]): RtDecider {
  const delays = createRng("delays");
  return {
    name: "slow rules",
    async decide(state, side, requests, config) {
      for (const request of requests) calls.push(`${state.time} ${side} ${request.unitId} ${request.point}`);
      await new Promise((resolve) => setTimeout(resolve, delays.int(4)));
      return ruleDecider.decide(state, side, requests, config);
    },
  };
}

async function playOut(handle: RunnerHandle, seconds?: number) {
  if (seconds != null) await handle.advance(seconds);
  else while (!handle.state.over) await handle.advance(300);
}

describe("RunnerHandle: in a worker and on this thread, the same game", () => {
  it("rules on both sides (decided in the worker): same final state, same feed, same entry contexts", async () => {
    for (const [list, seed] of [
      [SYMMETRIC_CONTROL_V1, "w0"],
      [SYMMETRIC_CONTROL_V1, "w1"],
      [COMBINED_ARMS_V1, "w2"],
    ] as const) {
      const s = spec(seed);
      const initial = opening(list, s);
      const contexts: Record<string, EntryContext[]> = { here: [], there: [] };
      const here = new InThreadRunner(initial, configFromSpec(s), { logLimit: 1e6, onEntry: (_e, c) => contexts.here.push(c) });
      const there = overChannel(s, initial, { logLimit: 1e6, onEntry: (_e, c) => contexts.there.push(c) });
      await playOut(here);
      await playOut(there);
      expect(there.state.over).toBeTruthy();
      expect(hash(there.state)).toBe(hash(here.state));
      expect(hash(there.log)).toBe(hash(here.log));
      expect(there.flags).toEqual(here.flags);
      expect(contexts.there).toEqual(contexts.here);
      expect(there.recentFor("blue")).toEqual(here.recentFor("blue"));
      there.dispose();
    }
  }, 60_000);

  it("deciders on the main thread answer the worker's questions, slowly, and the game is unchanged", async () => {
    const s = spec("w-proxy");
    const initial = opening(SYMMETRIC_CONTROL_V1, s);
    const asked = { here: [] as string[], there: [] as string[] };
    const here = new InThreadRunner(initial, configFromSpec(s), { deciders: { blue: slowRules(asked.here), red: slowRules(asked.here) } });
    const there = overChannel(s, initial, { deciders: { blue: slowRules(asked.there), red: slowRules(asked.there) } });
    await playOut(here);
    await playOut(there);
    // Every question went to the deciders here, and they were the same questions at the same moments.
    expect(asked.there.length).toBeGreaterThan(0);
    expect(asked.there).toEqual(asked.here);
    // (Fewer decisions than questions: a unit knocked out or broken before its answer is due takes none.)
    const decisions = (handle: RunnerHandle) => handle.log.filter((e: RtLogEntry) => e.type === "decision");
    expect(decisions(there).length).toBeGreaterThan(0);
    expect(decisions(there)).toEqual(decisions(here));
    expect(hash(there.state)).toBe(hash(here.state));
    there.dispose();
  }, 60_000);

  it("the replica is exact after every reply: lockstep, one simulated second at a time, Jev-style deciders on the main thread", async () => {
    for (const [list, seed] of [
      [SYMMETRIC_CONTROL_V1, "lock0"],
      [COMBINED_ARMS_V1, "lock1"],
    ] as const) {
      const s = spec(seed);
      const initial = opening(list, s);
      const here = new InThreadRunner(initial, configFromSpec(s), { deciders: { blue: slowRules([]), red: slowRules([]) } });
      const there = overChannel(s, initial, { deciders: { blue: slowRules([]), red: slowRules([]) } });
      let steps = 0;
      while (!here.state.over) {
        await here.advance(1);
        await there.advance(1);
        steps += 1;
        if (hash(there.state) !== hash(here.state)) throw new Error(`${seed}: replica differs at t=${here.state.time}`);
      }
      expect(steps).toBeGreaterThan(50);
      expect(there.state.over).toEqual(here.state.over);
      there.dispose();
    }
  }, 120_000);

  it("orders issued mid-game land the same way", async () => {
    const s = spec("w-orders");
    const initial = opening(SYMMETRIC_CONTROL_V1, s);
    const here = new InThreadRunner(initial, configFromSpec(s));
    const there = overChannel(s, initial, {});
    for (const handle of [here, there] as RunnerHandle[]) {
      await handle.advance(120);
      const result = heuristicOrders(handle.state, "blue");
      await handle.issueOrders(result.orders, { side: "blue", text: result.plan });
      await playOut(handle);
    }
    expect(there.state.plan?.blue).toBe(here.state.plan?.blue);
    expect(hash(there.state)).toBe(hash(here.state));
    expect(there.awaitingOrders).toEqual(here.awaitingOrders);
    there.dispose();
  }, 60_000);

  it("raster ground: the worker rebuilds the raster from its grid and plays the same game", async () => {
    const raster = await loadedRaster();
    const s = spec("w-raster", { source: "raster+relief", seed: "g1", origin: DEFAULT_ORIGIN, rasterRid: "ri.test.raster" });
    const initial = opening(SYMMETRIC_CONTROL_V1, s, raster);
    const here = new InThreadRunner(initial, configFromSpec(s, { raster }));
    const there = overChannel(s, initial, {}, raster);
    await playOut(here, 600);
    await playOut(there, 600);
    expect(hash(there.state)).toBe(hash(here.state));
    there.dispose();
  }, 60_000);

  it("reports a worker that cannot start, and refuses calls once stopped", async () => {
    const s = { ...spec("w-bad"), rulesetId: "no-such-rules" };
    const initial = opening(SYMMETRIC_CONTROL_V1, spec("w-bad"));
    const { port1, port2 } = new MessageChannel();
    port1.start();
    port2.start();
    const stopHost = hostRunner(port2 as unknown as MessageEndpoint);
    const bad = new WorkerRunner(port1 as unknown as MessageEndpoint, {
      spec: s,
      raster: null,
      initial,
      config: configFromSpec(spec("w-bad")),
    });
    await expect(bad.advance(10)).rejects.toThrow(/engine failed to start: Unknown ruleset/);
    stopHost();
    port1.close();
    port2.close();

    const there = overChannel(spec("w-stop"), initial, {});
    there.dispose();
    await expect(there.advance(10)).rejects.toThrow(/stopped/);
  });
});
