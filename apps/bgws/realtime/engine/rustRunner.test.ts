// The React screens on the Rust engine: translations, and a game played
// through RustRunner against a real bgws-server (skipped when it is not built:
// `cd bgws-rs && cargo build --release -p bgws-server`).

import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { configFromSpec } from "./configSpec";
import { applyEntities, logEntries, RustRunner, serverOrders, serverPhases } from "./rustRunner";
import { recordFromScenario } from "../headless";
import { mechCompanyAttack } from "../presets";
import type { UnitOrders } from "./types";
import { describeEntry, entryContext } from "./feed";
import { fireOverlay, newWrecks, radioOverlay, spottingOverlay, wreckOverlay } from "../overlays";
import { WORLD_LAYERS, worldOverlay } from "../worldOverlays";
import { projectForSide } from "../../lib/fogOfWar";

const scenario = mechCompanyAttack();
const record = recordFromScenario(scenario);

describe("RustRunner translations", () => {
  it("flattens order chains into the orders schema's phases", () => {
    const phases = serverPhases("go", { kind: "move", to: { lat: 54.2, lng: 18.6 }, mode: "tactical", then: { kind: "overwatch" } } as never);
    expect(phases).toEqual([
      { label: "go", do: "move", to: { lat: 54.2, lon: 18.6 }, mode: "tactical" },
      { label: "go (2)", do: "overwatch" },
    ]);
    const orders = { "b-tanks": { task: "hold", phases: [{ label: "a", order: { kind: "hold" } }, { label: "b", order: { kind: "observe" } }], phase: 1, intent: "x", urgency: "now", roe: "never", onContact: "observe", boundaries: [], by: "commander", issuedAt: 0 } } as unknown as Record<string, UnitOrders>;
    expect(serverOrders(orders)).toEqual([{ unit: "b-tanks", task: "hold", intent: "x", urgency: "now", roe: "never", onContact: "observe", phases: [{ label: "b", do: "observe" }] }]);
  });

  it("writes frames onto the state and turns events into the screens' entries", () => {
    const state = structuredClone(record.initial);
    const fe = state.game.forceElements["b-tanks"];
    applyEntities(state, [{ name: "b-tanks", lat: 54.3, lon: 18.7, heading_deg: 90, strength: 0.5, status: "pinned", suppression: 0.6 }]);
    expect(fe.position).toEqual({ lat: 54.3, lng: 18.7 });
    expect(fe.combatStrength).toBe(Math.round(fe.combatStrengthStart * 0.5));
    expect(fe.morale).toBe("suppressed2");
    expect(state.units["b-tanks"].suppression).toBe(60);
    applyEntities(state, [{ name: "b-tanks", lat: 54.3, lon: 18.7, heading_deg: 90, strength: 0, status: "destroyed", suppression: 0 }]);
    expect(fe.combatStrength).toBe(0);

    const shot = logEntries({ t: 12, kind: "fire.kill", text: "b-tanks fires", side: "blue", entity: "b-tanks", target: "r-ifv", data: { rounds: 4, hits: 2, kills: 1, rangeM: 1500, ph: 0.4 } }, state);
    expect(shot[0]).toMatchObject({ type: "shot", time: 12, side: "blue", shot: { firerId: "b-tanks", targetId: "r-ifv", knockedOut: 1, outcome: "knockedOut", rounds: 4 } });
    const decision = logEntries({ t: 5, kind: "decide.apply", text: "x", side: "red", entity: "r-atgm", target: null, data: { option: "fire", summary: "Fire now", source: { kind: "jev" }, options: [{ id: "fire", summary: "Fire now" }] } }, state);
    expect(decision[0]).toMatchObject({ type: "decision", decision: { unitId: "r-atgm", optionId: "fire", trace: { chosenBy: "jev", mark: "Fire now" } } });
    expect(logEntries({ t: 1, kind: "c2.planning", text: "HQ plans", side: "blue", entity: null, target: null, data: null }, state)[0]).toMatchObject({ type: "world", item: { kind: "c2.planning", text: "HQ plans" } });
    expect(logEntries({ t: 1, kind: "sim.start", text: "start", side: null, entity: null, target: null, data: null }, state)).toEqual([]);
  });
});

const ROOT = path.resolve(__dirname, "../../../..");
const SERVER = path.join(ROOT, "bgws-rs/target/release/bgws-server");
const PORT = 8790 + Math.floor(Math.random() * 100);

describe.skipIf(!existsSync(SERVER))("RustRunner against bgws-server", () => {
  let server: ChildProcess;
  beforeAll(async () => {
    server = spawn(SERVER, ["skirmish-symmetric-control-v1", "--port", String(PORT), "--deciders", "rules"], { cwd: path.join(ROOT, "bgws-rs"), stdio: "ignore" });
    for (let i = 0; i < 100; i++) {
      const up = await fetch(`http://127.0.0.1:${PORT}/`).then(() => true, () => false);
      if (up) return;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error("bgws-server did not start");
  }, 20_000);
  afterAll(() => {
    server?.kill();
  });

  it("plays the screen's scenario in the Rust engine through the screens' RunnerHandle", async () => {
    const entries: string[] = [];
    const runner = new RustRunner(`ws://127.0.0.1:${PORT}/ws`, record.initial, configFromSpec(record.spec), scenario, { onEntry: (e) => entries.push(e.type) });
    await runner.ready;
    expect(runner.where).toBe("bgws-server");
    const start = structuredClone(runner.state.game.forceElements);
    // Orders from the screen: the tanks pull back 1 km south.
    const tanks = start["b-tanks"].position;
    const back = { lat: tanks.lat - 0.009, lng: tanks.lng };
    await runner.issueOrders({ "b-tanks": { task: "withdraw", phases: [{ label: "back", order: { kind: "move", to: back, mode: "tactical" } }], phase: 0, intent: "keep the tanks", urgency: "now", roe: "always", onContact: "engage", boundaries: [], by: "commander", issuedAt: 0 } as unknown as UnitOrders });
    for (let i = 0; i < 10 && !runner.state.over; i++) await runner.advance(60);
    const s = runner.state;
    // Ten minutes, or a result before then (M2 often ends at red's breakpoint).
    expect(s.time >= 599 || !!s.over).toBe(true);
    expect(s.time).toBeGreaterThan(120);
    // Every unit the screen placed is in the Rust game, and they moved there.
    const moved = Object.values(s.game.forceElements).filter((fe) => fe.position.lat !== start[fe.id].position.lat || fe.position.lng !== start[fe.id].position.lng);
    expect(moved.length).toBeGreaterThan(3);
    expect(s.game.forceElements["b-tanks"].position.lat).toBeLessThan(tanks.lat - 0.002);
    // The feed: decisions and world items, and the screens' log.
    expect(entries).toContain("decision");
    expect(runner.log.length).toBeGreaterThan(10);
    expect(runner.recentFor("blue")).toBeInstanceOf(Array);
    // What the screen draws from it each frame: the same calls RealtimePlay's draw() makes.
    const view = { own: () => true, life: (s: number) => s };
    const shots = runner.log.filter((e) => e.type === "shot");
    const fire = fireOverlay(s, runner.log, view);
    if (shots.some((e) => s.time - e.time < 30)) expect(fire.lines.features.length).toBeGreaterThan(0);
    const wrecks = newWrecks(s, runner.log, -1);
    expect(wrecks.length).toBe(shots.reduce((n, e) => n + (e.type === "shot" && e.shot.knockedOut ? 1 : 0), 0));
    wreckOverlay(s, wrecks);
    spottingOverlay(s, runner.log, view);
    radioOverlay(s, runner.log, view);
    worldOverlay(s, { seenBy: "truth", layers: new Set(WORLD_LAYERS) });
    for (const entry of runner.log) {
      expect(describeEntry(entry).length).toBeGreaterThan(0);
      entryContext(entry, s);
    }
    for (const side of ["blue", "red"] as const) projectForSide(s.game, side);
    runner.dispose();
  }, 60_000);

  it("reports a scenario the server refuses", async () => {
    const bad = structuredClone(scenario);
    bad.forces[0].platform = "generic/no-such-thing";
    const runner = new RustRunner(`ws://127.0.0.1:${PORT}/ws`, record.initial, configFromSpec(record.spec), bad);
    await expect(runner.ready).rejects.toThrow(/refused|unknown platform/);
    runner.dispose();
  }, 20_000);
});
