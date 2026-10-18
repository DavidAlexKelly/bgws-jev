/** The map's overlays, drawn from a played game (overlays.ts). */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { scenarioFactory } from "../lib/forceBuilder";
import { flatTerrain } from "../lib/lineOfSight";
import { createRng } from "../rules/dice";
import { SYMMETRIC_CONTROL_V1 } from "../rules/forceList";
import { HOUSE_V1 } from "../rules/ruleset";
import { createRealtimeState } from "./engine/engine";
import { heuristicInitialOrders } from "./engine/orders";
import { RealtimeRunner, type RtLogEntry } from "./engine/runner";
import { DEFAULT_TIMING } from "./engine/timing";
import type { RtState } from "./engine/types";
import { fireOverlay, newWrecks, planOverlay, radioOverlay, spottingOverlay, wreckOverlay } from "./overlays";

beforeEach(() => {
  for (const k of ["groupCollapsed", "groupEnd", "log", "info", "table", "warn"] as const) {
    vi.spyOn(console, k).mockImplementation(() => {});
  }
});

const view = { own: () => true, life: (s: number) => s };

/** Play until `until` holds, keeping every feed entry. */
async function playUntil(until: (log: RtLogEntry[], state: RtState) => boolean) {
  const cfg = { ruleset: HOUSE_V1, terrain: flatTerrain(), rng: createRng("overlays"), timing: DEFAULT_TIMING, comms: "radio" as const };
  let state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
  state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
  const runner = new RealtimeRunner(state, cfg, { logLimit: 1e6 });
  while (!runner.state.over && !until(runner.log, runner.state)) await runner.advance(1);
  return runner;
}

describe("map overlays", () => {
  it("draws each volley by what it did, and a burst where it struck", async () => {
    const runner = await playUntil((log) => log.some((e) => e.type === "shot" && e.shot.outcome && e.shot.outcome !== "miss"));
    const { lines, impacts } = fireOverlay(runner.state, runner.log, view);
    expect(lines.features.length).toBeGreaterThan(0);
    expect(lines.features.every((f) => typeof f.properties.outcome === "string")).toBe(true);
    expect(impacts.features.length).toBeGreaterThan(0);
  }, 60_000);

  it("leaves a wreck where a vehicle was knocked out", async () => {
    const runner = await playUntil((log) => log.some((e) => e.type === "shot" && (e.shot.knockedOut ?? 0) > 0));
    const wrecks = newWrecks(runner.state, runner.log, -1);
    expect(wrecks.length).toBeGreaterThan(0);
    expect(wreckOverlay(runner.state, wrecks).features[0].properties.smoke).toBeGreaterThan(0);
  }, 60_000);

  it("marks sightings, and radio traffic from sender to hearers", async () => {
    const runner = await playUntil((log) => log.some((e) => e.type === "message"));
    // Right after it happened: both still visible.
    const sighted = runner.log.find((e) => e.type === "event" && e.event.kind === "sighted" && !e.event.reported)!;
    const atSighting = { ...runner.state, time: sighted.time };
    expect(spottingOverlay(atSighting, runner.log, view).lines.features.length).toBeGreaterThan(0);
    const { lines, pulses } = radioOverlay(runner.state, runner.log, view);
    expect(pulses.features.length).toBeGreaterThan(0);
    expect(lines.features[0].geometry.type).toBe("LineString");
  }, 60_000);

  it("draws the selected unit's plan: its route and the numbered steps after it", async () => {
    const runner = await playUntil(() => true);
    const id = Object.keys(runner.state.units).find((u) => runner.state.units[u].order.kind === "move")!;
    const plan = planOverlay(runner.state, id, view);
    expect(plan.lines.features.some((f) => f.properties.kind === "current")).toBe(true);
    // The move to the objective and the overwatch on it end in one place: one label.
    expect(plan.steps.map((s) => s.label)).toEqual(["1 · 2 overwatch"]);
    expect(planOverlay(runner.state, null, view).steps).toHaveLength(0);
  }, 60_000);
});
