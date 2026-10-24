// ── bench.test.ts ────────────────────────────────────────────────────────
// Step 0 of docs/plan/WORKER_AND_SCHEDULER.md: how the real-time engine's
// cost grows with the number of units, before anything is changed for scale.
//
// Skipped in CI (it is slow and its numbers are machine-dependent). Run with:
//   BGWS_BENCH=1 npx vitest run src/apps/bgws/realtime/engine/bench.test.ts
//
// For each size it plays two simulated minutes with the rules decider (no
// network), and prints wall time per simulated second and the scaling
// exponent between sizes (time ∝ n^k: k ≈ 1 linear, k ≈ 2 pairwise).

import { describe, expect, it } from "vitest";
import { scenarioFactory } from "../../lib/forceBuilder";
import { flatTerrain } from "../../lib/lineOfSight";
import { createRng } from "../../rules/dice";
import { SYMMETRIC_CONTROL_V1, type ForceList } from "../../rules/forceList";
import { HOUSE_V1 } from "../../rules/ruleset";
import { createRealtimeState } from "./engine";
import { heuristicInitialOrders } from "./orders";
import { RealtimeRunner } from "./runner";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig } from "./types";

const SIMULATED_S = 120;
/** Override with BGWS_BENCH_SIZES=300,600 to run only some sizes (e.g. when profiling). */
const SIZES = (process.env.BGWS_BENCH_SIZES ?? "20,100,300,600").split(",").map(Number);

/** The list repeated until it has about `target` units, each copy 1.5 km further along the front. */
function scaled(list: ForceList, target: number): ForceList {
  const copies = Math.max(1, Math.round(target / list.elements.length));
  const elements = Array.from({ length: copies }, (_, k) =>
    list.elements.map((e) => ({
      ...e,
      id: copies === 1 ? e.id : `${e.id}-x${k}`,
      offsetM: { east: e.offsetM.east + k * 1500, north: e.offsetM.north },
    })),
  ).flat();
  return { ...list, elements };
}

async function measure(target: number) {
  const list = scaled(SYMMETRIC_CONTROL_V1, target);
  const config: RtConfig = {
    ruleset: HOUSE_V1,
    terrain: flatTerrain(),
    rng: createRng(`bench:${target}`),
    // No breakpoint and a long limit: every size runs the full time, so sizes compare.
    timing: { ...DEFAULT_TIMING, breakpoint: 1, maxDurationS: 3600 },
  };
  let state = createRealtimeState(scenarioFactory(list, HOUSE_V1)());
  state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", config), "red", config);
  const runner = new RealtimeRunner(state, config, {});
  const start = runner.state.time;
  const t0 = performance.now();
  await runner.advance(SIMULATED_S);
  const ms = performance.now() - t0;
  const simulated = Math.max(1, runner.state.time - start);
  return { units: list.elements.length, ms, simulated, msPerSimS: ms / simulated, over: runner.state.over };
}

describe.skipIf(!process.env.BGWS_BENCH)("engine scaling benchmark", () => {
  it(
    "reports wall time per simulated second by force size",
    async () => {
      const rows: Awaited<ReturnType<typeof measure>>[] = [];
      for (const n of SIZES) rows.push(await measure(n));
      const lines = rows.map((r, i) => {
        const prev = rows[i - 1];
        const k = prev ? Math.log(r.msPerSimS / prev.msPerSimS) / Math.log(r.units / prev.units) : NaN;
        return `${String(r.units).padStart(5)} units  ${r.msPerSimS.toFixed(2).padStart(9)} ms/sim-s  ${(r.ms / 1000).toFixed(1).padStart(6)} s total  ${Number.isNaN(k) ? "      " : `k=${k.toFixed(2)}`}${r.over ? `  (ended at ${r.simulated.toFixed(0)} s)` : ""}`;
      });
      console.log(`\nEngine scaling, ${SIMULATED_S} simulated seconds each:\n${lines.join("\n")}\n`);
      expect(rows.every((r) => r.ms > 0)).toBe(true);
    },
    30 * 60 * 1000,
  );
});
