/**
 * The real-time mode: the clock, the autopilot, and when decisions are asked
 * for and take effect. No network: Jev is a fake that answers from a function.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { distanceM, metresPerDegreeLon, type LatLng } from "../../lib/board";
import { scenarioFactory } from "../../lib/forceBuilder";
import { flatTerrain, type TerrainSampler } from "../../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../../lib/state";
import { createRng } from "../../rules/dice";
import { SYMMETRIC_CONTROL_V1 } from "../../rules/forceList";
import type { JevAnswer, JevCall, JevRequest } from "../../rules/jev";
import { HOUSE_V1 } from "../../rules/ruleset";
import { ruleDecider, type RtDecider } from "./deciders";
import { createRealtimeState, setOrder, tick } from "./engine";
import { heuristicInitialOrders, jevInitialOrders } from "./initialOrders";
import { jevRealtimeDecider } from "./jevDecider";
import { optionsFor } from "./options";
import { RealtimeRunner } from "./runner";
import { DEFAULT_TIMING, perTick } from "./timing";
import type { RtConfig, RtEvent, RtState } from "./types";

beforeEach(() => {
  for (const k of ["groupCollapsed", "groupEnd", "log", "info", "table", "warn"] as const) {
    vi.spyOn(console, k).mockImplementation(() => {});
  }
});

// ── Fixtures ───────────────────────────────────────────────────────────────

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
function at(east: number, north: number): LatLng {
  return { lat: ORIGIN.lat + north / 111_320, lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat) };
}

function fe(id: string, side: Side, position: LatLng, extra: Partial<ForceElement> = {}): ForceElement {
  return {
    id,
    side,
    label: id,
    sidc: "SFGPUCA-------",
    moveType: "T",
    targetClass: "armoured_vehicle",
    capabilities: [{ kind: "atk", maxRangeM: 3000, shortRangeM: 1500 }],
    troopQuality: 4,
    combatStrength: 8,
    combatStrengthStart: 8,
    morale: "good",
    markers: [],
    concealed: false,
    isDummy: false,
    position,
    ...extra,
  };
}

function game(elements: ForceElement[], seen = false): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  if (seen) for (const e of elements) sighting[e.side === "blue" ? "red" : "blue"][e.id] = "full";
  return {
    gameId: "rt",
    scenarioId: "rt",
    turn: 1,
    phase: "arcAction",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((e) => [e.id, e])),
    sighting,
    objectives: { blue: at(0, 5000), red: at(0, -5000) },
    rng: { seed: "rt", cursor: 0 },
  };
}

function config(extra: Partial<RtConfig> = {}): RtConfig {
  return { ruleset: HOUSE_V1, terrain: flatTerrain(), rng: createRng("rt"), timing: DEFAULT_TIMING, ...extra };
}

function run(state: RtState, cfg: RtConfig, seconds: number) {
  const events: RtEvent[] = [];
  let shots = 0;
  for (let i = 0; i < seconds && !state.over; i += 1) {
    const r = tick(state, cfg);
    state = r.state;
    events.push(...r.events);
    shots += r.shots.length;
  }
  return { state, events, shots };
}

/** A fake Jev answering every choice with `pick(criteria)`, recording requests. */
function fakeJev(
  pick: (criteria: Record<string, string>, key: string) => string,
  confidence = 0.8,
): JevCall & { requests: JevRequest[] } {
  const requests: JevRequest[] = [];
  const call = (async (request: JevRequest) => {
    requests.push(request);
    const answers: Record<string, JevAnswer> = {};
    for (const [key, q] of Object.entries(request.questions)) {
      if (q.type !== "choice") continue;
      const choice = pick(q.criteria, key);
      answers[key] = { type: "choice", choice, probabilities: { [choice]: 0.8 }, confidence };
    }
    return { answers, latencyMs: 5 };
  }) as JevCall & { requests: JevRequest[] };
  call.requests = requests;
  return call;
}
const keyWhere = (criteria: Record<string, string>, pattern: RegExp) =>
  Object.keys(criteria).find((k) => pattern.test(criteria[k])) ?? "keep";

// ── Time ───────────────────────────────────────────────────────────────────

describe("time", () => {
  it("turns a per-turn chance into a per-second one that compounds back to it", () => {
    const p = perTick(0.5, DEFAULT_TIMING);
    expect(1 - Math.pow(1 - p, DEFAULT_TIMING.turnS)).toBeCloseTo(0.5, 6);
  });
});

// ── The autopilot ──────────────────────────────────────────────────────────

describe("the autopilot", () => {
  it("moves a unit at the ground's speed and stops it on arrival", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 500) }, cfg);
    const after10 = run(state, cfg, 10).state;
    const perSecond = HOUSE_V1.movement.T.open! / DEFAULT_TIMING.turnS;
    expect(distanceM(after10.game.forceElements.B1.position, at(0, 0))).toBeCloseTo(perSecond * 10, 0);

    const { state: done, events } = run(state, cfg, 2000);
    expect(distanceM(done.game.forceElements.B1.position, at(0, 500))).toBeLessThan(1);
    expect(done.units.B1.order.kind).toBe("hold");
    expect(events.some((e) => e.kind === "arrived" && e.unitId === "B1")).toBe(true);
  });

  it("never enters ground it cannot cross: it goes as far as it can and stops", () => {
    // "steep" has no allowance for tracks: impassable.
    const lake: TerrainSampler = {
      groundHeightM: () => 0,
      classify: (p) => (p.lat > at(0, 100).lat ? "steep" : "open"),
    };
    const cfg = config({ terrain: lake });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, -30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000) }, cfg);
    const { state: after, events } = run(state, cfg, 600);
    expect(events.some((e) => e.unitId === "B1" && (e.kind === "blocked" || e.kind === "arrived"))).toBe(true);
    expect(after.game.forceElements.B1.position.lat).toBeLessThanOrEqual(at(0, 101).lat);
    expect(after.units.B1.order.kind).toBe("hold");
  });

  it("sights what is in view, and says so", () => {
    const cfg = config();
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))]));
    const { state: after, events } = run(state, cfg, 300);
    expect(after.game.sighting.blue.R1).not.toBe("none");
    expect(events.some((e) => e.kind === "sighted" && e.unitId === "B1")).toBe(true);
  });

  it("lets contact fade when nobody can see it any more", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2000))], true));
    // R1 drives out of sight range; blue's picture of it goes stale.
    state = setOrder(state, "R1", { kind: "move", to: at(0, 12_000) }, cfg);
    const { state: after } = run(state, cfg, 1800);
    expect(after.game.sighting.blue.R1 ?? "none").toBe("none");
  });

  it("fires once per engagement cycle, and not at all under 'never'", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    state = setOrder(state, "B1", { kind: "overwatch" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const { shots } = run(state, cfg, DEFAULT_TIMING.engagementCycleS * 2 + 1);
    expect(shots).toBeGreaterThanOrEqual(2);
    expect(shots).toBeLessThanOrEqual(3);

    let quiet = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    quiet = setOrder(quiet, "B1", { kind: "hold" }, cfg, { roe: "never" });
    quiet = setOrder(quiet, "R1", { kind: "hold" }, cfg, { roe: "never" });
    expect(run(quiet, cfg, 600).shots).toBe(0);
  });

  it("resolves fire simultaneously: both sides shoot in the same second", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 800))], true));
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    const first = tick(state, cfg);
    expect(first.shots.map((s) => s.firerId).sort()).toEqual(["B1", "R1"]);
  });

  it("does not favour whichever side comes first in the list", async () => {
    const wins = { blue: 0, red: 0, draw: 0 };
    for (let i = 0; i < 40; i += 1) {
      const cfg = config({ rng: createRng(`bias${i}`) });
      let state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
      state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
      const runner = new RealtimeRunner(state, cfg);
      while (!runner.state.over) await runner.advance(300);
      wins[runner.state.over?.winner ?? "draw"] += 1;
    }
    // Identical forces on open ground: neither side should run away with it.
    expect(Math.abs(wins.blue - wins.red)).toBeLessThanOrEqual(14);
  }, 120_000);

  it("is deterministic: the same seed gives the same game", async () => {
    const play = async () => {
      const cfg = config({ rng: createRng("same") });
      let state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
      state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
      const runner = new RealtimeRunner(state, cfg);
      while (!runner.state.over) await runner.advance(300);
      return JSON.stringify(runner.state.game.forceElements) + runner.state.time;
    };
    expect(await play()).toBe(await play());
  }, 60_000);
});

// ── Meeting the enemy ──────────────────────────────────────────────────────

describe("meeting the enemy", () => {
  const column = (roe: "never" | "withinShortRange" = "withinShortRange") => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 4000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 6000) }, cfg, { roe });
    state = setOrder(state, "R1", { kind: "move", to: at(0, -2000) }, cfg, { roe });
    return { cfg, state };
  };

  it("fires on the move rather than driving past", () => {
    const { cfg, state } = column();
    const { shots } = run(state, cfg, 900);
    expect(shots).toBeGreaterThan(0);
  });

  it("halts on running into the enemy, and says so", () => {
    const { cfg, state } = column("never");
    const { state: after, events } = run(state, cfg, 900);
    const contact = events.find((e) => e.kind === "contact");
    expect(contact).toBeDefined();
    // Nobody drove through: they are still on their own sides of each other.
    expect(after.game.forceElements.B1.position.lat).toBeLessThan(after.game.forceElements.R1.position.lat);
    expect(distanceM(after.game.forceElements.B1.position, after.game.forceElements.R1.position)).toBeGreaterThan(200);
  });

  it("sees anything close in the open without a roll", () => {
    const cfg = config();
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 400))]));
    const after = tick(state, cfg).state;
    expect(after.game.sighting.blue.R1).toBe("full");
  });

  it("the rules engage what they sight instead of carrying on", async () => {
    const cfg = config();
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    const moving = setOrder(state, "B1", { kind: "move", to: at(0, 3000) }, cfg);
    const [decision] = await ruleDecider.decide(
      moving,
      "blue",
      [
        {
          unitId: "B1",
          events: [{ time: 1, unitId: "B1", kind: "sighted", detail: "R1", severe: true }],
          options: optionsFor(moving, "B1", cfg),
        },
      ],
      cfg,
    );
    expect(decision.optionId).toBe("engage:R1");
  });
});

// ── Routes ─────────────────────────────────────────────────────────────────

describe("routes", () => {
  it("follows the planner's waypoints instead of a straight line", () => {
    // A planner that goes round by the east.
    const planner = {
      kind: "raster" as const,
      plan: (_from: LatLng, to: LatLng) => [at(1000, 0), at(1000, 1000), to],
    };
    const cfg = config({ planner });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, -30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000) }, cfg);
    let furthestEast = 0;
    for (let i = 0; i < 2000 && state.units.B1.order.kind === "move"; i += 1) {
      state = tick(state, cfg).state;
      const east = distanceM(ORIGIN, { lat: ORIGIN.lat, lng: state.game.forceElements.B1.position.lng });
      furthestEast = Math.max(furthestEast, east);
    }
    expect(furthestEast).toBeGreaterThan(900);
    expect(distanceM(state.game.forceElements.B1.position, at(0, 1000))).toBeLessThan(1);
  });

  it("never steps onto ground the raster says is impassable, even where the land cover would let it ford", () => {
    const river = (p: LatLng) => !(p.lat > at(0, 200).lat && p.lat < at(0, 260).lat);
    const cfg = config({ isPassable: river, planner: { kind: "raster", plan: () => null } });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, -30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000) }, cfg);
    const { state: after } = run(state, cfg, 900);
    expect(after.game.forceElements.B1.position.lat).toBeLessThanOrEqual(at(0, 200).lat);
  });
});

// ── When decisions are asked and take effect ───────────────────────────────

describe("the runner", () => {
  /** A decider that records when it was asked and answers `optionId`. */
  function recording(optionId: string, delayMs = 0): RtDecider & { calls: { time: number; units: string[]; kinds: string[] }[] } {
    const calls: { time: number; units: string[]; kinds: string[] }[] = [];
    return {
      name: "recording",
      calls,
      async decide(state, _side, requests) {
        calls.push({
          time: state.time,
          units: requests.map((r) => r.unitId),
          kinds: requests.flatMap((r) => r.events.map((e) => e.kind)),
        });
        if (delayMs) await new Promise((resolve) => setTimeout(resolve, delayMs));
        return requests.map((r) => ({
          unitId: r.unitId,
          optionId: r.options.some((o) => o.id === optionId) ? optionId : "keep",
          trace: { question: "", options: [], chosenId: optionId, chosenBy: "jev" as const },
        }));
      },
    };
  }

  it("asks when something happens, and applies the answer after the reaction time", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 5000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 200) }, cfg);
    const decider = recording("objective");
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(400);

    const arrivedAt = runner.log.find((e) => e.type === "event" && e.event.kind === "arrived")!.time;
    const decision = runner.log.find((e) => e.type === "decision")!;
    expect(decider.calls[0].kinds).toContain("arrived");
    // Asked after the coalescing window; applied after B1's reaction time.
    expect(decider.calls[0].time).toBe(arrivedAt + DEFAULT_TIMING.coalesceS);
    expect(decision.time).toBe(decider.calls[0].time + DEFAULT_TIMING.reactionS(4));
    expect(runner.state.units.B1.order.kind).toBe("move");
  });

  it("gives the same game however slowly the decider answers", async () => {
    const play = async (delayMs: number) => {
      const cfg = config({ rng: createRng("slow") });
      let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1400))]));
      state = setOrder(state, "B1", { kind: "move", to: at(0, 600) }, cfg);
      const runner = new RealtimeRunner(state, cfg, {
        deciders: { blue: recording("overwatch", delayMs), red: recording("overwatch", delayMs) },
      });
      await runner.advance(900);
      return { game: JSON.stringify(runner.state.game.forceElements), waits: runner.waits };
    };
    const fast = await play(0);
    const slow = await play(30);
    expect(slow.game).toBe(fast.game);
    expect(slow.waits).toBeGreaterThan(0);
  }, 30_000);

  it("asks a side's units together, and does not nag a unit inside its cooldown", async () => {
    const cfg = config();
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(200, 0)), fe("R1", "red", at(0, 1200))]),
    );
    state = setOrder(state, "R1", { kind: "overwatch" }, cfg);
    const decider = recording("keep");
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(600);
    // Every batch is one call; no unit appears twice within the cooldown
    // unless something severe happened to it.
    for (const call of decider.calls) expect(new Set(call.units).size).toBe(call.units.length);
    const askedB1 = decider.calls.filter((c) => c.units.includes("B1"));
    for (let i = 1; i < askedB1.length; i += 1) {
      const gap = askedB1[i].time - askedB1[i - 1].time;
      const severe = askedB1[i].kinds.some((k) => k === "hit" || k === "friendLost" || k === "sighted" || k === "moraleDrop");
      if (!severe) expect(gap).toBeGreaterThanOrEqual(DEFAULT_TIMING.cooldownS);
    }
  });
});

// ── Jev in command ─────────────────────────────────────────────────────────

describe("Jev in real time", () => {
  const scene = () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(200, 0)), fe("R1", "red", at(0, 1000))], true));
    state = setOrder(state, "B1", { kind: "hold" }, cfg);
    return { cfg, state };
  };
  const requests = (state: RtState, cfg: RtConfig) =>
    ["B1", "B2"].map((unitId) => ({
      unitId,
      events: [{ time: 0, unitId, kind: "sighted" as const, detail: "R1 at 1000 m", severe: false }],
      options: optionsFor(state, unitId, cfg),
    }));

  it("asks for a side's units in ONE request and takes Jev's orders", async () => {
    const { cfg, state } = scene();
    const call = fakeJev((criteria) => keyWhere(criteria, /engage/));
    const decisions = await jevRealtimeDecider({ side: "blue", call }).decide(state, "blue", requests(state, cfg), cfg);
    expect(call.requests).toHaveLength(1);
    expect(Object.keys(call.requests[0].questions)).toEqual(["u0", "u1"]);
    expect(decisions.map((d) => d.optionId)).toEqual(["engage:R1", "engage:R1"]);
    expect(decisions[0].trace.chosenBy).toBe("jev");
  });

  it("carries on when Jev is unsure", async () => {
    const { cfg, state } = scene();
    const call = fakeJev((criteria) => keyWhere(criteria, /engage/), 0.05);
    const decisions = await jevRealtimeDecider({ side: "blue", call }).decide(state, "blue", requests(state, cfg), cfg);
    expect(decisions.every((d) => d.optionId === "keep" && d.trace.fallback === "lowConfidence")).toBe(true);
  });

  it("lets the rules decide when Jev cannot be reached", async () => {
    const { cfg, state } = scene();
    const failing: JevCall = async () => {
      throw new Error("down");
    };
    const decisions = await jevRealtimeDecider({ side: "blue", call: failing }).decide(state, "blue", requests(state, cfg), cfg);
    const byRules = await ruleDecider.decide(state, "blue", requests(state, cfg), cfg);
    expect(decisions.map((d) => d.optionId)).toEqual(byRules.map((d) => d.optionId));
    expect(decisions[0].trace.fallback).toBe("error");
  });

  it("never tells Jev about an enemy its side has not seen", async () => {
    const cfg = config();
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], false));
    const call = fakeJev(() => "keep");
    await jevRealtimeDecider({ side: "blue", call }).decide(
      state,
      "blue",
      [{ unitId: "B1", events: [], options: optionsFor(state, "B1", cfg) }],
      cfg,
    );
    expect(JSON.stringify(call.requests[0].state)).not.toContain("R1");
  });

  it("plays a whole game with Jev in command on both sides", async () => {
    const cfg = config({ rng: createRng("whole") });
    let state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
    const opinion = (seed: string) => {
      const rng = createRng(seed);
      return fakeJev((criteria) => {
        const keys = Object.keys(criteria);
        return keys[rng.int(keys.length)];
      }, 0.6);
    };
    state = await jevInitialOrders(state, "blue", cfg, opinion("ib"));
    state = await jevInitialOrders(state, "red", cfg, opinion("ir"));
    const holder: { runner: RealtimeRunner | null } = { runner: null };
    const runner = new RealtimeRunner(state, cfg, {
      deciders: {
        blue: jevRealtimeDecider({ side: "blue", call: opinion("b") }, () => holder.runner?.recentFor("blue") ?? []),
        red: jevRealtimeDecider({ side: "red", call: opinion("r") }, () => holder.runner?.recentFor("red") ?? []),
      },
    });
    holder.runner = runner;
    while (!runner.state.over) await runner.advance(300);
    expect(runner.log.some((e) => e.type === "decision" && e.decision.trace.chosenBy === "jev")).toBe(true);
    expect(runner.state.over).toBeDefined();
  }, 60_000);
});

// ── Initial orders ─────────────────────────────────────────────────────────

describe("initial orders", () => {
  it("heuristic: everyone advances on the objective, with a purpose", () => {
    const cfg = config();
    const state = heuristicInitialOrders(
      createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 9000))])),
      "blue",
      cfg,
    );
    expect(state.units.B1.order.kind).toBe("move");
    const order = state.units.B1.order as { to: LatLng; route?: LatLng[] };
    expect(distanceM(order.to, at(0, 5000))).toBeLessThan(5);
    expect(order.route?.length).toBeGreaterThan(0);
    expect(state.units.B1.purpose).toBe("take the objective");
  });

  it("Jev: one request choosing each unit's opening order and rules of engagement", async () => {
    const cfg = config();
    const base = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(200, 0)), fe("R1", "red", at(0, 9000))]));
    const call = fakeJev((criteria, key) =>
      key.endsWith("_roe") ? "never" : keyWhere(criteria, /overwatch/),
    );
    const state = await jevInitialOrders(base, "blue", cfg, call);
    expect(call.requests).toHaveLength(1);
    expect(state.units.B1.order.kind).toBe("overwatch");
    expect(state.units.B1.roe).toBe("never");
  });

  it("Jev: falls back to the heuristic when unreachable, so the game can still start", async () => {
    const cfg = config();
    const base = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 9000))]));
    const state = await jevInitialOrders(base, "blue", cfg, async () => {
      throw new Error("down");
    });
    expect(state.units.B1.order.kind).toBe("move");
  });
});
