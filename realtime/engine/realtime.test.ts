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
import { COMBINED_ARMS_V1, SYMMETRIC_CONTROL_V1 } from "../../rules/forceList";
import type { JevAnswer, JevCall, JevRequest } from "../../rules/jev";
import { HOUSE_V1 } from "../../rules/ruleset";
import { ruleDecider, type RtDecider, type RtDecisionRequest } from "./deciders";
import { decisionPointOf, optionsAt, ruleFallback, type DecisionContext } from "./decisions";
import { createRealtimeState, setOrder, tick } from "./engine";
import { heuristicInitialOrders } from "./orders";
import { jevRealtimeDecider } from "./jevDecider";
import { damageEffect, describeEffect, oddsAgainst } from "./options";
import { penetrationAt, strikeOdds } from "./lethality";
import { hullDownAgainst, hullDownSpot, platformSpeedFactor, slopeFactor } from "./geometry";
import { acquisitionS, aimedIntervalS, errorBudget, hitChance } from "./fire";
import { detectChance, detectionRate } from "./detection";
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
    // A Challenger-like troop: peers mostly bounce off each other's fronts,
    // as the real ones do, so a fight lasts long enough to test.
    capabilities: [{ kind: "atk", munition: "ke", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 650 }],
    armour: { frontKeMm: 700, turretFrontKeMm: 950, sideKeMm: 140, rearKeMm: 40, roofKeMm: 30 },
    armourMm: 700,
    facing: side === "blue" ? 0 : 180,
    platformCount: 4,
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

/** The question a unit would be asked about this event: its decision point, options and the rules' fallback. */
function ask(
  state: RtState,
  unitId: string,
  kind: RtEvent["kind"],
  cfg: RtConfig,
  extra: Partial<RtEvent> = {},
  context: DecisionContext = {},
): RtDecisionRequest {
  const event: RtEvent = { time: state.time, unitId, kind, detail: kind, severe: false, ...extra };
  const found = decisionPointOf([event]);
  if (!found) throw new Error(`${kind} is not a decision point`);
  const options = optionsAt(state, unitId, found.point, event, cfg, context);
  return { unitId, point: found.point, event, events: [event], options, fallback: ruleFallback(state, unitId, found.point, options) };
}

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
    state = setOrder(state, "B1", { kind: "move", to: at(0, 500), mode: "march" }, cfg);
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
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000), mode: "march" }, cfg);
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
    state = setOrder(state, "R1", { kind: "move", to: at(0, 12_000), mode: "tactical" }, cfg);
    const { state: after } = run(state, cfg, 1800);
    expect(after.game.sighting.blue.R1 ?? "none").toBe("none");
  });

  it("fires once per shot interval, and not at all under 'never'", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    state = setOrder(state, "B1", { kind: "overwatch" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const interval = aimedIntervalS(state.game.forceElements.B1.capabilities[0], DEFAULT_TIMING.shotIntervalS);
    const { shots } = run(state, cfg, interval * 2 + 1);
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
    // Both lay on at the same speed, so the first rounds go in the same second.
    let first = tick(state, cfg);
    for (let i = 0; i < 60 && first.shots.length === 0; i += 1) first = tick(first.state, cfg);
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
    state = setOrder(state, "B1", { kind: "move", to: at(0, 6000), mode: "tactical" }, cfg, { roe });
    state = setOrder(state, "R1", { kind: "move", to: at(0, -2000), mode: "tactical" }, cfg, { roe });
    return { cfg, state };
  };

  it("fires on the move rather than driving past", () => {
    const { cfg, state } = column();
    const { shots } = run(state, cfg, 900);
    expect(shots).toBeGreaterThan(0);
  });

  it("halts on running into the enemy, and says so", () => {
    const { cfg, state: start } = column("never");
    // Until they meet, however fast the ground lets them close.
    let after = start;
    let contact: RtEvent | undefined;
    for (let s = 0; s < 3600 && !contact && !after.over; s += 1) {
      const r = tick(after, cfg);
      after = r.state;
      contact = r.events.find((e) => e.kind === "contact");
    }
    expect(contact).toBeDefined();
    // Nobody drove through: they are still on their own sides of each other.
    expect(after.game.forceElements.B1.position.lat).toBeLessThan(after.game.forceElements.R1.position.lat);
    expect(distanceM(after.game.forceElements.B1.position, after.game.forceElements.R1.position)).toBeGreaterThan(200);
  });

  it("sees anything close in the open without a roll; a friend hears after the report delay", () => {
    const cfg = config();
    // B2 is behind a ridge of distance: it cannot see R1 itself.
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(0, -2900)), fe("R1", "red", at(0, 400))]));
    // Nobody fires: the test is about seeing, not about who wins.
    for (const id of ["B1", "B2", "R1"]) state = setOrder(state, id, { kind: "hold" }, cfg, { roe: "never" });
    const after = tick(state, cfg).state;
    expect(after.units.B1.ownSeen.R1?.level).toBe("full");
    expect(after.units.B2.picture.R1).toBeUndefined();
    const reported = run(after, cfg, DEFAULT_TIMING.reportDelayS).state;
    expect(reported.units.B2.picture.R1).toMatchObject({ level: "full", from: "B1" });
  });

  it("the rules engage what they sight instead of carrying on", async () => {
    const cfg = config();
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    const moving = setOrder(state, "B1", { kind: "move", to: at(0, 3000), mode: "tactical" }, cfg);
    const [decision] = await ruleDecider.decide(moving, "blue", [ask(moving, "B1", "sighted", cfg, { about: "R1", severe: true })], cfg);
    expect(decision.optionId).toBe("engage");
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
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000), mode: "tactical" }, cfg);
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
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000), mode: "tactical" }, cfg);
    const { state: after } = run(state, cfg, 900);
    expect(after.game.forceElements.B1.position.lat).toBeLessThanOrEqual(at(0, 200).lat);
  });
});

// ── When decisions are asked and take effect ───────────────────────────────

describe("the runner", () => {
  /** A decider that records when it was asked and answers `optionId`. */
  type Call = { time: number; units: string[]; kinds: string[]; severe: string[] };
  function recording(optionId: string, delayMs = 0): RtDecider & { calls: Call[] } {
    const calls: Call[] = [];
    return {
      name: "recording",
      calls,
      async decide(state, _side, requests) {
        calls.push({
          time: state.time,
          units: requests.map((r) => r.unitId),
          kinds: requests.flatMap((r) => r.events.map((e) => e.kind)),
          severe: requests.filter((r) => r.events.some((e) => e.severe)).map((r) => r.unitId),
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
    state = setOrder(state, "B1", { kind: "move", to: at(0, 200), mode: "march" }, cfg);
    const decider = recording("resume");
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(400);

    const arrivedAt = runner.log.find((e) => e.type === "event" && e.event.kind === "arrived")!.time;
    const decision = runner.log.find((e) => e.type === "decision")!;
    expect(decider.calls[0].kinds).toContain("arrived");
    // Asked after the coalescing window; applied after B1's reaction time.
    expect(decider.calls[0].time).toBe(arrivedAt + DEFAULT_TIMING.coalesceS);
    expect(decision.time).toBe(decider.calls[0].time + DEFAULT_TIMING.reactionS(4));
    // Arrived is decision point D10; "resume" took it back to its mission ground.
    expect(decision.type === "decision" && decision.summary).toMatch(/back to its mission/);
  });

  it("gives the same game however slowly the decider answers", async () => {
    const play = async (delayMs: number) => {
      const cfg = config({ rng: createRng("slow") });
      let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1400))]));
      state = setOrder(state, "B1", { kind: "move", to: at(0, 600), mode: "march" }, cfg);
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
      const severe = askedB1[i].severe.includes("B1");
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
    ["B1", "B2"].map((unitId) => ask(state, unitId, "sighted", cfg, { about: "R1", detail: "R1 at 1000 m" }));

  it("asks for a side's units in ONE request and takes Jev's orders", async () => {
    const { cfg, state } = scene();
    const call = fakeJev((criteria) => keyWhere(criteria, /engage/));
    const decisions = await jevRealtimeDecider({ side: "blue", call }).decide(state, "blue", requests(state, cfg), cfg);
    expect(call.requests).toHaveLength(1);
    expect(Object.keys(call.requests[0].questions)).toEqual(["u0", "u1"]);
    expect(decisions.map((d) => d.optionId)).toEqual(["engage", "engage"]);
    expect(decisions[0].trace.chosenBy).toBe("jev");
  });

  it("lets the rules decide when Jev is unsure, rather than carrying on", async () => {
    const { cfg, state } = scene();
    const call = fakeJev((criteria) => keyWhere(criteria, /overwatch/), 0.05);
    const decisions = await jevRealtimeDecider({ side: "blue", call }).decide(state, "blue", requests(state, cfg), cfg);
    const byRules = await ruleDecider.decide(state, "blue", requests(state, cfg), cfg);
    expect(decisions.map((d) => d.optionId)).toEqual(byRules.map((d) => d.optionId));
    expect(decisions.every((d) => d.trace.fallback === "lowConfidence")).toBe(true);
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
    await jevRealtimeDecider({ side: "blue", call }).decide(state, "blue", [ask(state, "B1", "arrived", cfg)], cfg);
    expect(JSON.stringify(call.requests[0])).not.toContain("R1");
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
    state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
    const runner = new RealtimeRunner(state, cfg, {
      deciders: {
        blue: jevRealtimeDecider({ side: "blue", call: opinion("b") }),
        red: jevRealtimeDecider({ side: "red", call: opinion("r") }),
      },
    });
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
    expect(state.units.B1.mission).toMatchObject({ task: "take", purpose: "take and hold the objective" });
    // Mission orders: a task in phases, carried out one after another.
    expect(state.units.B1.orders?.phases.map((p) => p.order.kind)).toEqual(["move", "overwatch"]);
    expect(state.units.B1.order.phase).toBe(0);
  });

});

// ── Realism: suppression, breaking, rallying, missions ────────────────────

/** A unit that has lost vehicles: `fit` of `total` still fighting, strength to match. */
function losing(state: RtState, id: string, fit: number, total: number): RtState {
  const fe = state.game.forceElements[id];
  const strength = Math.max(1, Math.round((fe.combatStrengthStart * fit) / total));
  return {
    ...state,
    game: { ...state.game, forceElements: { ...state.game.forceElements, [id]: { ...fe, combatStrength: strength } } },
    units: { ...state.units, [id]: { ...state.units[id], vehicles: { total, fit } } },
  };
}

describe("suppression and nerve", () => {
  it("builds suppression from fire, misses included, and lets it fade once the fire stops", () => {
    // Every round misses: misses alone suppress, and nobody breaks and ends the game.
    const cfg = config({ timing: { ...DEFAULT_TIMING, strikeScale: 0 } });
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(100, 0)), fe("B3", "blue", at(-100, 0)), fe("R1", "red", at(0, 1500))], true),
    );
    for (const id of ["B1", "B2", "B3"]) state = setOrder(state, id, { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const fired = run(state, cfg, 25).state;
    expect(fired.units.R1.suppression).toBeGreaterThan(0);
    // Stop the fire and wait: it fades back to nothing.
    let quiet = fired;
    for (const id of ["B1", "B2", "B3"]) quiet = setOrder(quiet, id, { kind: "hold" }, cfg, { roe: "never" });
    const later = run(quiet, cfg, 120).state;
    expect(later.units.R1.suppression).toBe(0);
    expect(later.game.forceElements.R1.morale).toBe("good");
  });

  it("does not break a unit per hit, but tests it at a loss threshold", () => {
    const cfg = config();
    // A defender at 25% losses: under its 40% threshold, no test.
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = losing(state, "B1", 3, 4);
    expect(tick(state, cfg).state.units.B1.breakTests).toBe(0);
    // Past it, a hopeless crew breaks and a superb one holds.
    const hurt = (tq: number) => {
      const s = createRealtimeState(game([fe("B1", "blue", at(0, 0), { troopQuality: tq }), fe("R1", "red", at(0, 30_000))]));
      return losing(s, "B1", 2, 4);
    };
    const bad = tick(hurt(-8), cfg).state.units.B1;
    expect(bad.cohesion).toBe("broken");
    expect(bad.order.kind).toBe("withdraw");
    const good = tick(hurt(20), cfg).state.units.B1;
    expect(good.cohesion).toBe("steady");
    expect(good.breakTests).toBe(1);
  });

  it("a broken unit falls back ONCE, stops, rallies, and is asked again", async () => {
    const cfg = config();
    let state = createRealtimeState(
      game([
        fe("B1", "blue", at(0, 0), { troopQuality: -8 }),
        fe("B2", "blue", at(0, -600)),
        fe("B3", "blue", at(300, -600)),
        fe("R1", "red", at(0, 30_000)),
      ]),
    );
    state = losing(state, "B1", 2, 4);
    state = tick(state, cfg).state;
    expect(state.units.B1.cohesion).toBe("broken");
    // Now make it a good crew, so the rally is certain; the break stays.
    state = { ...state, game: { ...state.game, forceElements: { ...state.game.forceElements, B1: { ...state.game.forceElements.B1, troopQuality: 20 } } } };
    const withdrawals: number[] = [];
    const events: RtEvent[] = [];
    for (let i = 0; i < 1200 && !state.over; i += 1) {
      const before = state.units.B1.order.kind;
      const r = tick(state, cfg);
      state = r.state;
      events.push(...r.events);
      if (state.units.B1.order.kind === "withdraw" && before !== "withdraw") withdrawals.push(state.time);
    }
    expect(state.over).toBeUndefined();
    expect(withdrawals).toHaveLength(0); // never sent back again after the first
    expect(distanceM(state.game.forceElements.B1.position, at(0, -600))).toBeLessThan(5); // rallied on its friends
    expect(state.units.B1.cohesion).toBe("steady");
    expect(events.some((e) => e.unitId === "B1" && e.kind === "rallied")).toBe(true);
  });

  it("the runner does not ask shaken or broken units", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, cohesion: "shaken", lastRallyCheckAt: 1e9 } } };
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    const asked: string[] = [];
    const runner = new RealtimeRunner(state, cfg, {
      deciders: {
        blue: {
          name: "spy",
          async decide(_s, _side, requests) {
            asked.push(...requests.map((r) => r.unitId));
            return requests.map((r) => ({ unitId: r.unitId, optionId: "keep", trace: { question: "", options: [], chosenId: "keep", chosenBy: "heuristic" as const } }));
          },
        },
      },
    });
    await runner.advance(120);
    expect(asked).not.toContain("B1");
  });
});

describe("missions", () => {
  it("a quiet unit off its mission is asked again, and the rules resume it", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = setOrder(state, "B1", { kind: "hold" }, cfg, { mission: { task: "take", at: at(0, 3000), purpose: "take the objective" } });
    const runner = new RealtimeRunner(state, cfg);
    await runner.advance(DEFAULT_TIMING.idleS + 30);
    expect(runner.log.some((e) => e.type === "event" && e.event.kind === "idle")).toBe(true);
    expect(runner.state.units.B1.order.kind).toBe("move");
  });

  it("a unit on its mission is left alone", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = setOrder(state, "B1", { kind: "overwatch" }, cfg);
    expect(run(state, cfg, 600).events.some((e) => e.kind === "idle")).toBe(false);
  });

  it("offers following up a broken enemy (D7), and the rules get on with the orders", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1000))], true));
    state = setOrder(state, "B1", { kind: "overwatch" }, cfg, { mission: { task: "take", at: at(0, 3000), purpose: "take the objective" } });
    state = { ...state, units: { ...state.units, R1: { ...state.units.R1, cohesion: "broken" } } };
    const request = ask(state, "B1", "enemyBroke", cfg, { about: "R1" });
    expect(request.point).toBe("D7");
    expect(request.options.map((o) => o.id)).toEqual(["resume", "watch", "regain"]);
    expect(request.options.find((o) => o.id === "regain")!.order).toMatchObject({ kind: "move", mode: "assault" });
    const [decision] = await ruleDecider.decide(state, "blue", [request], cfg);
    expect(decision.optionId).toBe("resume");
  });

  it("ends the game when a side is past its breakpoint, not at its last unit", () => {
    const cfg = config();
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(500, 0)), fe("R1", "red", at(0, 30_000)), fe("R2", "red", at(500, 30_000))]),
    );
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, cohesion: "broken" } } };
    const after = tick(state, cfg).state;
    expect(after.over?.winner).toBe("red");
    expect(after.over?.reason).toMatch(/breakpoint/);
  });
});

describe("the crew's drills and movement", () => {
  const woods: TerrainSampler = {
    groundHeightM: () => 0,
    // A wood 100 m east of the start line.
    classify: (p) => (p.lng > at(80, 0).lng && p.lng < at(160, 0).lng ? "woodsLight" : "open"),
  };

  it("reacts to contact at once: returns fire and dashes for the nearest cover", () => {
    const cfg = config({ terrain: woods });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2500))], true));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 2000), mode: "tactical" }, cfg);
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    // R1 lays on B1 (acquisition time), fires, and B1's drill runs that second.
    let after = state;
    let events: RtEvent[] = [];
    for (let i = 0; i < 60 && !events.some((e) => e.unitId === "B1" && e.kind === "underFire"); i += 1) {
      const r = tick(after, cfg);
      after = r.state;
      events = r.events;
    }
    const underFire = events.find((e) => e.unitId === "B1" && e.kind === "underFire");
    expect(underFire?.detail).toMatch(/dashing .* for cover/);
    expect(after.units.B1.order).toMatchObject({ kind: "move", dash: true });
    expect(after.units.B1.weaponReadyAt).toBeLessThanOrEqual(after.time + 5);
  });

  it("an assault presses on under fire and closes to point-blank", () => {
    // Nothing lands, so the test is about the movement, not who wins the fire fight.
    const cfg = config({ terrain: woods, timing: { ...DEFAULT_TIMING, strikeScale: 0 } });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1200))], true));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1200), mode: "assault" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const { events, state: after } = run(state, cfg, 900);
    const contact = events.find((e) => e.kind === "contact" && e.unitId === "B1");
    expect(contact?.detail).toMatch(/closed with/);
    expect(distanceM(after.game.forceElements.B1.position, after.game.forceElements.R1.position)).toBeLessThanOrEqual(151);
  });

  it("moves faster on a road march than tactically, and does not fire on the march", () => {
    const cfg = config();
    const go = (mode: "march" | "tactical") => {
      let s = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
      s = setOrder(s, "B1", { kind: "move", to: at(0, 5000), mode }, cfg);
      return distanceM(run(s, cfg, 60).state.game.forceElements.B1.position, at(0, 0));
    };
    expect(go("tactical") / go("march")).toBeCloseTo(0.6, 1);

    let s = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2500))], true));
    s = setOrder(s, "B1", { kind: "move", to: at(0, 30_000), mode: "march" }, cfg, { roe: "always" });
    s = setOrder(s, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const shots = [];
    for (let i = 0; i < 30; i += 1) {
      const r = tick(s, cfg);
      s = r.state;
      shots.push(...r.shots.filter((shot) => shot.firerId === "B1"));
    }
    expect(shots).toHaveLength(0);
  });

  it("bounding overwatch: a pair never both moves at once", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(200, 0)), fe("R1", "red", at(0, 30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 3000), mode: "bound" }, cfg);
    state = setOrder(state, "B2", { kind: "move", to: at(200, 3000), mode: "bound" }, cfg);
    let both = 0;
    let moved = 0;
    for (let i = 0; i < 900; i += 1) {
      state = tick(state, cfg).state;
      const m1 = state.units.B1.lastMovedAt === state.time;
      const m2 = state.units.B2.lastMovedAt === state.time;
      if (m1 && m2) both += 1;
      if (m1 || m2) moved += 1;
    }
    expect(moved).toBeGreaterThan(100);
    expect(both).toBe(0);
  });

  it("defends itself against whoever fires on it, even outside its rules of engagement", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2500))], true));
    // B1 only fires inside short range (1500 m); R1 at 2500 m fires first.
    state = setOrder(state, "B1", { kind: "hold" }, cfg, { roe: "withinShortRange" });
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    const { state: after } = run(state, cfg, 60);
    expect(after.units.R1.attackers.B1).toBeDefined();
  });

  it("keeps a last-known position once contact fades", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2000))], true));
    state = setOrder(state, "R1", { kind: "move", to: at(0, 12_000), mode: "march" }, cfg);
    const { state: after } = run(state, cfg, 1800);
    expect(after.game.sighting.blue.R1 ?? "none").toBe("none");
    expect(after.lastKnown.blue.R1).toBeDefined();
    expect(distanceM(after.lastKnown.blue.R1.at, after.game.forceElements.R1.position)).toBeGreaterThan(1000);
  });
});

describe("decisiveness", () => {
  it("games end on a breakpoint, not the time limit or an endless retreat", async () => {
    let limit = 0;
    let rallyPointArrivals = 0;
    let breaks = 0;
    for (let i = 0; i < 12; i += 1) {
      const cfg = config({ rng: createRng(`decisive${i}`) });
      let state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
      state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
      const runner = new RealtimeRunner(state, cfg, { logLimit: 1e6 });
      while (!runner.state.over) await runner.advance(300);
      if (runner.state.over?.reason === "time limit") limit += 1;
      for (const entry of runner.log) {
        if (entry.type !== "event") continue;
        if (entry.event.kind === "moraleDrop" && entry.event.detail.startsWith("broken")) breaks += 1;
        if (entry.event.kind === "arrived" && entry.event.detail === "reached its rally point") rallyPointArrivals += 1;
      }
    }
    expect(limit).toBe(0);
    // Each break sends a unit back once at most.
    expect(rallyPointArrivals).toBeLessThanOrEqual(breaks);
  }, 120_000);
});


// ── Context for Jev: memory, honest odds, coordination ─────────────────────

describe("context for decisions", () => {
  /** Two blue troops trading long-range fire with one red one — the scene that kept "carrying on". */
  const longRange = (strikeScale = 0) => {
    const cfg = config({ timing: { ...DEFAULT_TIMING, strikeScale } });
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(300, 0)), fe("R1", "red", at(0, 2800)), fe("R9", "red", at(0, 40_000))], true),
    );
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "B2", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    return { cfg, state };
  };

  it("keeps a record of each unit's fire, and raises 'ineffective' when it is doing nothing", () => {
    const { cfg, state: start } = longRange(0);
    // R1 holds its fire, so B1 is not pinned (and handed to the autopilot) first.
    const state = setOrder(start, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const { state: after, events } = run(state, cfg, 200);
    const e = after.units.B1.engagement!;
    expect(e.targetId).toBe("R1");
    expect(e.shots).toBeGreaterThanOrEqual(Math.floor(180 / aimedIntervalS(state.game.forceElements.B1.capabilities[0], DEFAULT_TIMING.shotIntervalS)));
    expect(e.damage).toBe(0);
    const ineffective = events.find((ev) => ev.unitId === "B1" && ev.kind === "ineffective");
    expect(ineffective?.detail).toMatch(/fire not working .* shots/);
    expect(after.units.R1.incoming.B1.shots).toBeGreaterThan(0);
  });

  it("shows the real chance of doing damage, in words, with the figures kept for the console", () => {
    const { cfg, state } = longRange(DEFAULT_TIMING.strikeScale);
    const engage = ask(state, "R1", "sighted", cfg, { about: "B2" }).options.find((o) => o.id === "engage")!;
    expect(engage.summary).toMatch(/to knock out one of its vehicles within a minute/);
    expect(engage.summary).not.toMatch(/\d+%/);
    expect(engage.exact).toMatch(/%\/min to knock out one of its vehicles/);
    const odds = oddsAgainst(state.game.forceElements.R1, state.game.forceElements.B2, state, cfg)!;
    const effect = damageEffect(state.game.forceElements.R1, state.game.forceElements.B2, state, cfg)!;
    // A hit is not a knock-out: per round, the chance of one is the hit chance × getting through × killing.
    expect(effect.strike!.pKnockOut).toBeLessThan(1);
    expect(effect.perShot).toBeLessThan(1 - Math.pow(1 - odds.pHit, odds.rounds));
  });

  it("when fire is not working (D8), offers a better shot and the rules take it", async () => {
    const { cfg, state: start } = longRange(0);
    const { state } = run(start, cfg, 200);
    // Judged with rounds landing; the run had none land, to guarantee the misses.
    const judged = { ...cfg, timing: DEFAULT_TIMING };
    const request = ask(state, "B1", "ineffective", judged);
    expect(request.point).toBe("D8");
    expect(request.options.map((o) => o.id)).toEqual(expect.arrayContaining(["better", "quiet", "keep"]));
    const better = request.options.find((o) => o.id === "better")!;
    expect(better.order).toMatchObject({ kind: "move", mode: "tactical", then: { kind: "engage", targetId: "R1" } });
    expect(better.effect!).toBeGreaterThan(damageEffect(state.game.forceElements.B1, state.game.forceElements.R1, state, judged)?.perMinute ?? 0);
    const [decision] = await ruleDecider.decide(state, "blue", [request], judged);
    expect(decision.optionId).toBe("better");
  });

  it("gives Jev each unit's memory and the side's picture of who is fighting whom", async () => {
    const { cfg, state: start } = longRange(0);
    const runner = new RealtimeRunner(start, cfg);
    await runner.advance(400);
    const state = runner.state;
    // The runner remembered its decisions, "carry on" included.
    expect(state.units.B1.history.length).toBeGreaterThan(0);

    const call = fakeJev(() => "keep", 0.9);
    const requests = ["B1", "B2"].map((unitId) => ask(state, unitId, "ineffective", cfg, { detail: "fire not working" }));
    await jevRealtimeDecider({ side: "blue", call }).decide(state, "blue", requests, cfg);
    const sent = call.requests[0];
    const b1 = (sent.state as { units: Record<string, unknown>[] }).units.find((u) => u.id === "B1")!;
    expect(b1.lastDecisions).toBeDefined();
    expect(b1.doesTheEnemyKnowYouAreHere).toBeDefined();
    const r1 = (b1.enemies as Record<string, unknown>[]).find((e) => e.id === "R1")!;
    expect(r1).toMatchObject({ belief: "identified", range: "very long range" });
    expect(r1.itsFireOnYou).toMatch(/within a minute|cannot|no chance/);
    // Friends nearby, and what they are doing, so units can work together.
    expect((b1.friendsNearby as { id: string }[]).map((f) => f.id)).toContain("B2");
    const question = (sent.questions.u0 as { instructions: string }).instructions;
    expect(question).toMatch(/Decision point D8/);
    expect(question).toMatch(/The intent comes first/);
    expect(question).toMatch(/Also deciding now.*B2/);
    // Words, not numbers.
    expect(JSON.stringify(sent.state)).not.toMatch(/\d+ ?m\b|\d+%/);
  });

  it("keeps a whole-battle request within Jev's context", async () => {
    const cfg = config({ rng: createRng("size") });
    let state = createRealtimeState(scenarioFactory(COMBINED_ARMS_V1, HOUSE_V1)());
    state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
    const runner = new RealtimeRunner(state, cfg);
    await runner.advance(420);
    const call = fakeJev(() => "keep");
    const blue = Object.values(runner.state.game.forceElements).filter((one) => one.side === "blue" && one.combatStrength > 0);
    await jevRealtimeDecider({ side: "blue", call }).decide(
      runner.state,
      "blue",
      blue.map((one) => ask(runner.state, one.id, "idle", cfg)),
      cfg,
    );
    const size = JSON.stringify(call.requests[0]).length;
    if (process.env.SHOW_SIZE) process.stdout.write(`whole-side request: ${size} characters\n`);
    // Jev's window is 32k tokens; ~4 characters a token, with room for the answer.
    expect(size).toBeLessThan(90_000);
  }, 60_000);
});


// ── Range ──────────────────────────────────────────────────────────────────

describe("range", () => {
  it("makes point-blank fire far more accurate and lethal than long-range fire", () => {
    const cfg = config();
    const state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 300)), fe("R2", "red", at(0, 2900))], true),
    );
    const [b1, r1, r2] = ["B1", "R1", "R2"].map((id) => state.game.forceElements[id]);
    const close = oddsAgainst(b1, r1, state, cfg)!;
    const far = oddsAgainst(b1, r2, state, cfg)!;
    expect(close.pHit).toBeGreaterThan(far.pHit + 0.15);
    const closeEffect = damageEffect(b1, r1, state, cfg)!;
    const farEffect = damageEffect(b1, r2, state, cfg)!;
    expect(farEffect.minutesToKnockOut!).toBeGreaterThan(closeEffect.minutesToKnockOut! * 1.5);
  });

  it("works hit chance out from an error budget: range, motion, cover and the gun's own ballistics", () => {
    const t = DEFAULT_TIMING;
    const g = scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)();
    const tank = g.forceElements["red-1"];
    const gun = g.forceElements["blue-1"].capabilities.find((c) => c.kind === "atk")!;
    const p = (range: number, c = {}) => hitChance(gun, tank, range, { aspect: "front", ...c }, t);
    // Stationary, modern fire control: near-certain close in, falling continuously with range.
    expect(p(1000)).toBeGreaterThan(0.95);
    expect(p(2000)).toBeGreaterThan(0.85);
    expect(p(1500)).toBeGreaterThan(p(2000));
    expect(p(2000)).toBeGreaterThan(p(2500));
    expect(p(2500)).toBeGreaterThan(p(3000));
    // Each degrades it, more at range.
    for (const c of [{ targetMoving: true }, { firerMoving: true }, { targetHullDown: true, aspect: "turret" }, { targetInCover: true }, { firerSuppressed: true }]) {
      expect(p(2500, c)).toBeLessThan(p(2500));
    }
    // A slow round leads a moving target worse than a fast one: flight time from muzzle velocity.
    const heat = { ...gun, munition: "ce" as const, muzzleVelocityMs: 900 };
    expect(errorBudget(heat, 2000, {}).timeOfFlightS).toBeGreaterThan(errorBudget(gun, 2000, {}).timeOfFlightS * 1.5);
    expect(hitChance(heat, tank, 2000, { aspect: "front", targetMoving: true }, t)).toBeLessThan(p(2000, { targetMoving: true }));
    // Rate of fire from the data: the Challenger's gun sustains 6 aimed rounds a minute.
    expect(aimedIntervalS(gun, t.shotIntervalS)).toBe(10);
    expect(hitChance(gun, tank, 600, {}, { ...t, strikeScale: 0 })).toBe(0);
  });

  it("says what a shot did: a hit that did no damage is not reported as a hit", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2000))], true));
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const results = new Set<string>();
    for (let i = 0; i < 900 && !state.over; i += 1) {
      const r = tick(state, cfg);
      state = r.state;
      for (const shot of r.shots) results.add(shot.result);
    }
    for (const result of results) {
      expect(result).toMatch(
        /^\d+ rounds?, (all missed|\d+ hit: (did not penetrate \(.+\)|penetrated \(.+\), crew fighting on|intercepted by active protection|knocked out \d+ \(.+\); \d+\/\d+ left))$/,
      );
    }
  });

  it("a better shot closes to inside a kilometre, where fire pays", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2800))], true));
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, engagement: { targetId: "R1", since: 0, lastShotAt: 0, shots: 18, hits: 0, damage: 0, window: { since: 0, shots: 18, hits: 0, damage: 0 } } } } };
    const better = ask(state, "B1", "ineffective", cfg, { about: "R1" }).options.find((o) => o.id === "better")!;
    expect(better.order.kind).toBe("move");
    const to = (better.order as { to: LatLng }).to;
    expect(distanceM(to, state.game.forceElements.R1.position)).toBeLessThanOrEqual(1000);
  });
});


// ── From the data: vehicles, penetration, hull-down, speed ─────────────────

describe("the data behind a shot", () => {
  const challenger = () => {
    const g = scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)();
    return { blue: g.forceElements["blue-1"], red: g.forceElements["red-1"] };
  };

  it("reads penetration off the munition's curve at the actual range", () => {
    const { blue } = challenger();
    const gun = blue.capabilities.find((c) => c.kind === "atk")!;
    expect(penetrationAt(gun, 0)).toBe(676);
    expect(penetrationAt(gun, 1500)).toBeCloseTo(638.5, 1);
    expect(penetrationAt(gun, 3000)).toBe(583);
    // No curve: the 1 km figure, kinetic falloff beyond it; a shaped charge does not fall off.
    expect(penetrationAt({ kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 500 }, 2000)).toBeCloseTo(470, 5);
    expect(penetrationAt({ kind: "atk", munition: "ce", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 500 }, 2000)).toBe(500);
  });

  it("makes the face struck decide the outcome: a Challenger's front mostly stops a peer round, its side does not", () => {
    const { blue, red } = challenger();
    const gun = blue.capabilities.find((c) => c.kind === "atk")!;
    const facingSouth = { ...red, position: at(0, 0), facing: 180 };
    const front = strikeOdds(gun, facingSouth, at(0, -1500), 1500, HOUSE_V1);
    const side = strikeOdds(gun, facingSouth, at(1500, 0), 1500, HOUSE_V1);
    const hullDown = strikeOdds(gun, facingSouth, at(0, -1500), 1500, HOUSE_V1, true);
    expect(front.aspect).toBe("front");
    expect(front.armourMm).toBe(700);
    expect(front.pPenetrate).toBeLessThan(0.4);
    expect(side.aspect).toBe("side");
    expect(side.pPenetrate).toBeGreaterThan(0.99);
    expect(hullDown.aspect).toBe("turret");
    expect(hullDown.pPenetrate).toBeLessThan(0.1);
    expect(hullDown.pPenetrate).toBeLessThan(front.pPenetrate / 4);
  });

  it("knocks out vehicles one at a time, and strength follows", () => {
    const cfg = config();
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(200, 0)), fe("R1", "red", at(0, 400), { platformCount: 4 })], true),
    );
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "B2", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    expect(state.units.R1.vehicles).toEqual({ total: 4, fit: 4 });
    const seen: number[] = [state.units.R1.vehicles.fit];
    const labels: string[] = [];
    for (let i = 0; i < 1200 && state.game.forceElements.R1.combatStrength > 0 && !state.over; i += 1) {
      const r = tick(state, cfg);
      state = r.state;
      labels.push(...r.shots.map((s) => s.result));
      if (seen[seen.length - 1] !== state.units.R1.vehicles.fit) seen.push(state.units.R1.vehicles.fit);
    }
    // It loses vehicles one knock-out at a time (it may break, and red lose, before the last).
    expect(seen[0]).toBe(4);
    expect(seen.length).toBeGreaterThan(1);
    const { fit, total } = state.units.R1.vehicles;
    const r1 = state.game.forceElements.R1;
    expect(r1.combatStrength).toBe(fit === 0 ? 0 : Math.max(1, Math.round((r1.combatStrengthStart * fit) / total)));
    expect(labels.some((l) => /knocked out \d/.test(l))).toBe(true);
  });

  it("finds hull-down positions from the ground itself", () => {
    // A low crest 60-100 m north of the start line: it hides a hull, not a turret.
    const crest: TerrainSampler = {
      groundHeightM: (p) => {
        const north = (p.lat - ORIGIN.lat) * 111_320;
        return north > 60 && north < 100 ? 1.8 : 0;
      },
      classify: () => "open",
    };
    const cfg = config({ terrain: crest });
    expect(hullDownAgainst(at(0, 0), at(0, 2000), cfg)).toBe(true);
    // Not from the side, and not on open ground.
    expect(hullDownAgainst(at(0, 0), at(2000, 0), cfg)).toBe(false);
    expect(hullDownAgainst(at(0, -400), at(0, 2000), config())).toBe(false);
    const spot = hullDownSpot(fe("B1", "blue", at(0, -150)), at(0, 2000), cfg, 200);
    expect(spot).not.toBeNull();
    expect(hullDownAgainst(spot!, at(0, 2000), cfg)).toBe(true);
  });

  it("moves each platform at its own speed, and slower uphill for a weaker engine", () => {
    const fast = { ...fe("B1", "blue", at(0, 0)), speedKmh: 72 };
    const slow = { ...fe("B2", "blue", at(0, 0)), speedKmh: 45 };
    expect(platformSpeedFactor(fast)).toBeCloseTo(1.2, 5);
    expect(platformSpeedFactor(slow)).toBeCloseTo(0.75, 5);
    expect(platformSpeedFactor(fe("B3", "blue", at(0, 0)))).toBe(1);
    const hill: TerrainSampler = { groundHeightM: (p) => (p.lat - ORIGIN.lat) * 111_320 * 0.1, classify: () => "open" };
    const cfg = config({ terrain: hill });
    const strong = slopeFactor({ ...fast, hpPerTonne: 27 }, at(0, 0), at(0, 50), cfg);
    const weak = slopeFactor({ ...fast, hpPerTonne: 14 }, at(0, 0), at(0, 50), cfg);
    expect(strong).toBeLessThan(1);
    expect(weak).toBeLessThan(strong);
    expect(slopeFactor(fast, at(0, 50), at(0, 0), cfg)).toBe(1);
  });

  it("tells Jev which face it would strike and whether rounds get through", () => {
    const cfg = config();
    const state = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
    const effect = damageEffect(state.game.forceElements["blue-1"], state.game.forceElements["red-1"], state, cfg)!;
    expect(describeEffect(effect)).toMatch(/knock out one of its vehicles \((front|side|rear), \d+ vs \d+ mm: \d+% penetrate\)/);
  });
});

// ── What slows a real engagement: spotting, acquisition, fire distribution ──

describe("finding, laying on and sharing out targets", () => {
  it("spots at a rate: faster close, moving or firing; slower in cover, hull-down or on the move", () => {
    const base = detectionRate(1000, {});
    expect(detectionRate(500, {})).toBeCloseTo(base * 4, 6);
    expect(detectionRate(1000, { targetMoving: true })).toBeGreaterThan(base);
    expect(detectionRate(1000, { targetFired: true })).toBeGreaterThan(base);
    expect(detectionRate(1000, { targetInCover: true })).toBeLessThan(base);
    expect(detectionRate(1000, { targetHullDown: true })).toBeLessThan(base);
    expect(detectionRate(1000, { observerMoving: true })).toBeLessThan(base);
    expect(detectChance(100, {}, 1)).toBe(1);
  });

  it("takes longer to find a still enemy far off than a close one", () => {
    const timeToSight = (north: number) => {
      let total = 0;
      for (let seed = 0; seed < 20; seed += 1) {
        const cfg = config({ rng: createRng(`spot${seed}`) });
        let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, north))]));
        state = setOrder(state, "B1", { kind: "hold" }, cfg, { roe: "never" });
        state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
        let t = 0;
        while (t < 600 && !state.units.B1.ownSeen.R1) {
          state = tick(state, cfg).state;
          t += 1;
        }
        total += t;
      }
      return total / 20;
    };
    expect(timeToSight(2500)).toBeGreaterThan(timeToSight(600) * 3);
  });

  it("lays on a new target before the first round, but not on one it is already engaging", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2000))], true));
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const shotTimes: number[] = [];
    for (let i = 0; i < 60; i += 1) {
      const r = tick(state, cfg);
      state = r.state;
      for (const shot of r.shots) shotTimes.push(shot.time);
    }
    // In sight of the crew itself (tracked from the first second), 2 km off: 4 + 4 s.
    expect(shotTimes[0]).toBeGreaterThanOrEqual(acquisitionS(2000));
    expect(acquisitionS(2000, { onlyReported: true })).toBeGreaterThan(acquisitionS(2000));
    // After that, the weapon's own rate of fire.
    const gun = state.game.forceElements.B1.capabilities[0];
    expect(shotTimes[1] - shotTimes[0]).toBe(aimedIntervalS(gun, DEFAULT_TIMING.shotIntervalS));
  });

  it("wastes rounds when two crews pick the same tank, or a fresh wreck", () => {
    const cfg = config();
    let state = createRealtimeState(
      game(
        [
          fe("B1", "blue", at(0, 0)),
          fe("B2", "blue", at(200, 0)),
          fe("R1", "red", at(0, 300), { platformCount: 1, armour: { frontKeMm: 10, sideKeMm: 10, rearKeMm: 10 } }),
          fe("R2", "red", at(3000, 30_000)),
        ],
        true,
      ),
    );
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "B2", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const labels: string[] = [];
    for (let i = 0; i < 60 && state.game.forceElements.R1.combatStrength > 0; i += 1) {
      const r = tick(state, cfg);
      state = r.state;
      labels.push(...r.shots.map((s) => s.result));
    }
    // Eight rounds at one thin-skinned vehicle at 300 m: it dies, and the rest were wasted on it.
    expect(state.game.forceElements.R1.combatStrength).toBe(0);
    expect(labels.some((l) => /on a tank already knocked out/.test(l))).toBe(true);
  });
});
