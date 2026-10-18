/**
 * The command design (docs/REALTIME_COMMAND_DESIGN.html): what each unit
 * knows, the decision points, triggers, and mission orders from the
 * commander. No network: the commander and Jev are fakes.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { bearingDeltaDeg, distanceM, metresPerDegreeLon, type LatLng } from "../../lib/board";
import { flatTerrain } from "../../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../../lib/state";
import { createRng } from "../../rules/dice";
import { HOUSE_V1 } from "../../rules/ruleset";
import type { RtDecider, RtDecisionRequest } from "./deciders";
import { decisionPointOf, optionsAt, ruleFallback, triggersFor, type DecisionContext } from "./decisions";
import { createRealtimeState, setOrder, tick } from "./engine";
import { beliefOf, chanceLocatedAfter, locateChance, selfBeliefOf } from "./knowledge";
import { applyOrders, commanderOrders, heuristicOrders, ordersPrompt, parseOrders } from "./orders";
import { RealtimeRunner } from "./runner";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig, RtEvent, RtState, UnitOrders } from "./types";

beforeEach(() => {
  for (const k of ["groupCollapsed", "groupEnd", "log", "info", "table", "warn"] as const) {
    vi.spyOn(console, k).mockImplementation(() => {});
  }
});

// ── Fixtures (as realtime.test.ts) ─────────────────────────────────────────

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
  return { ruleset: HOUSE_V1, terrain: flatTerrain(), rng: createRng("cmd"), timing: DEFAULT_TIMING, ...extra };
}
const noHits = (extra: Partial<RtConfig> = {}) => config({ timing: { ...DEFAULT_TIMING, strikeScale: 0 }, ...extra });

/** Tick until `until` holds or `limit` seconds pass; every event on the way. */
function runUntil(state: RtState, cfg: RtConfig, until: (s: RtState, events: RtEvent[]) => boolean, limit = 600) {
  const events: RtEvent[] = [];
  const shots: { time: number; firerId: string }[] = [];
  for (let i = 0; i < limit && !state.over; i += 1) {
    const r = tick(state, cfg);
    state = r.state;
    events.push(...r.events);
    shots.push(...r.shots);
    if (until(state, r.events)) break;
  }
  return { state, events, shots };
}

function ask(state: RtState, unitId: string, kind: RtEvent["kind"], cfg: RtConfig, extra: Partial<RtEvent> = {}, context: DecisionContext = {}): RtDecisionRequest {
  const event: RtEvent = { time: state.time, unitId, kind, detail: kind, severe: false, ...extra };
  const { point } = decisionPointOf([event])!;
  const options = optionsAt(state, unitId, point, event, cfg, context);
  return { unitId, point, event, events: [event], options, fallback: ruleFallback(state, unitId, point, options) };
}

function ordersOf(phases: UnitOrders["phases"], extra: Partial<UnitOrders> = {}): UnitOrders {
  return {
    task: phases.map((p) => p.label).join(", then "),
    phases,
    phase: 0,
    intent: "test intent",
    urgency: "whenAble",
    roe: "withinShortRange",
    onContact: "engage",
    boundaries: [],
    by: "claude",
    issuedAt: 0,
    ...extra,
  };
}

/** A decider that records what it was asked and answers with `pick`. */
function recording(pick: (request: RtDecisionRequest) => string = (r) => r.fallback): RtDecider & { asked: RtDecisionRequest[] } {
  const asked: RtDecisionRequest[] = [];
  return {
    name: "recording",
    asked,
    async decide(_state, _side, requests) {
      asked.push(...requests);
      return requests.map((r) => ({
        unitId: r.unitId,
        optionId: pick(r),
        trace: { question: `${r.point}: x`, options: [], chosenId: pick(r), chosenBy: "jev" as const },
      }));
    },
  };
}

// ── What each unit knows ───────────────────────────────────────────────────

describe("knowledge: the locate roll", () => {
  it("is certain point-blank, about even at a kilometre in the open, and poor far off", () => {
    expect(locateChance(120, { volleys: 1 })).toBe(1);
    expect(locateChance(1000, { volleys: 1 })).toBeGreaterThan(0.4);
    expect(locateChance(1000, { volleys: 1 })).toBeLessThan(0.6);
    expect(locateChance(2500, { volleys: 1 })).toBeLessThan(0.15);
  });

  it("is harder against a shooter hull-down or in cover, easier with every volley and when searching", () => {
    const base = locateChance(1500, { volleys: 1 });
    expect(locateChance(1500, { volleys: 1, shooterHullDown: true })).toBeLessThan(base);
    expect(locateChance(1500, { volleys: 1, shooterInCover: true })).toBeLessThan(base);
    expect(locateChance(1500, { volleys: 3 })).toBeGreaterThan(base);
    expect(locateChance(1500, { volleys: 1, targetSearching: true })).toBeGreaterThan(base);
    // The firer's own estimate after several volleys rises with each.
    expect(chanceLocatedAfter(2000, 4, {})).toBeGreaterThan(chanceLocatedAfter(2000, 1, {}));
  });

  it("no longer gives a shooter away automatically: the target learns a bearing and rolls", () => {
    let unlocated = 0;
    for (let seed = 0; seed < 6; seed += 1) {
      const cfg = noHits({ rng: createRng(`locate${seed}`) });
      let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2800))]));
      state = setOrder(state, "B1", { kind: "hold" }, cfg, { roe: "never" });
      // R1 already knows where B1 is; B1 does not know R1.
      state = { ...state, units: { ...state.units, R1: { ...state.units.R1, ownSeen: { B1: { time: 0, level: "full" } } } } };
      state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
      const { state: after, events } = runUntil(state, cfg, (_s, ev) => ev.some((e) => e.kind === "underFire"), 120);
      const fired = events.find((e) => e.kind === "underFire" && e.unitId === "B1");
      if (!fired || fired.located) continue;
      unlocated += 1;
      expect(fired.detail).toMatch(/shooter not located/);
      expect(bearingDeltaDeg(fired.bearingDeg!, 0)).toBeLessThan(5);
      expect(beliefOf(after, "B1", "R1")).toBe("suspected");
      expect(after.units.B1.locating.R1.volleys).toBe(1);
      expect(decisionPointOf([fired])?.point).toBe("D3");
    }
    expect(unlocated).toBeGreaterThan(0);
  });

  it("the drill takes cover but does not return fire at a shooter it has not located", () => {
    for (let seed = 0; seed < 6; seed += 1) {
      const cfg = noHits({ rng: createRng(`drill${seed}`) });
      let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2800))]));
      state = setOrder(state, "B1", { kind: "move", to: at(0, 2000), mode: "tactical" }, cfg);
      state = { ...state, units: { ...state.units, R1: { ...state.units.R1, ownSeen: { B1: { time: 0, level: "full" } } } } };
      state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
      const { events } = runUntil(state, cfg, (_s, ev) => ev.some((e) => e.kind === "underFire"), 120);
      const fired = events.find((e) => e.kind === "underFire" && e.unitId === "B1");
      if (fired && fired.located === false) {
        expect(fired.detail).not.toMatch(/returning fire/);
        return;
      }
    }
    throw new Error("every seed located the shooter at once");
  });

  it("knows a unit has been seen once it is fired on", () => {
    const cfg = noHits();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1500))], true));
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    expect(selfBeliefOf(state.units.B1, state.time)).toBe("unobserved");
    const { state: after } = runUntil(state, cfg, (s) => s.units.B1.lastIncomingAt === s.time, 60);
    expect(selfBeliefOf(after.units.B1, after.time)).toBe("knownSeen");
  });
});

describe("knowledge: cues", () => {
  it("a hidden unit reads an enemy halting as a sign it may have been seen, at most once in 30 s", () => {
    const cfg = noHits();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1600))], true));
    state = setOrder(state, "B1", { kind: "wait", targetId: "R1", trigger: { kind: "range", withinM: 300 }, autoFire: false }, cfg, { roe: "never" });
    // R1, already on the move, drives 60 m more and stops: by coincidence, but B1 cannot know that.
    state = setOrder(state, "R1", { kind: "move", to: at(0, 1540), mode: "tactical" }, cfg, { roe: "never" });
    state = { ...state, units: { ...state.units, R1: { ...state.units.R1, lastMovedAt: state.time } } };
    const { state: after, events } = runUntil(state, cfg, () => false, 60);
    const cues = events.filter((e) => e.kind === "cue" && e.unitId === "B1");
    expect(cues).toHaveLength(1);
    expect(cues[0].detail).toMatch(/R1 halted/);
    expect(decisionPointOf(cues)?.point).toBe("D5");
    expect(selfBeliefOf(after.units.B1, after.time)).toBe("possiblySeen");
    // What it may do about it: under weapons hold, not "fire first".
    const request = ask(after, "B1", "cue", cfg, { about: "R1" });
    expect(request.options[0].id).toBe("keep");
    expect(request.options.map((o) => o.id)).not.toContain("fireFirst");
  });
});

// ── Triggers ───────────────────────────────────────────────────────────────

describe("waiting for a trigger", () => {
  const approach = (autoFire: boolean) => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2000))], true));
    state = setOrder(state, "B1", { kind: "wait", targetId: "R1", trigger: { kind: "range", withinM: 800 }, autoFire }, cfg);
    state = setOrder(state, "R1", { kind: "move", to: at(0, -3000), mode: "march" }, cfg, { roe: "never" });
    return { cfg, state };
  };

  it("holds fire until the trigger, then fires at once when briefed to", () => {
    const { cfg, state } = approach(true);
    const { state: after, events, shots } = runUntil(state, cfg, (s) => s.units.B1.lastShotAt === s.time, 600);
    const met = events.find((e) => e.kind === "triggerMet");
    expect(met?.info).toBe(true); // no decision needed
    const first = shots.find((s) => s.firerId === "B1")!;
    expect(first.time - met!.time).toBeLessThanOrEqual(1); // laid on already, as an ambush is
    expect(distanceM(after.game.forceElements.B1.position, after.game.forceElements.R1.position)).toBeLessThanOrEqual(820);
  });

  it("otherwise asks (D4): fire now, wait for a closer shot, or let it pass", () => {
    const { cfg, state } = approach(false);
    const { state: after, events, shots } = runUntil(state, cfg, (_s, ev) => ev.some((e) => e.kind === "triggerMet"), 600);
    const met = events.find((e) => e.kind === "triggerMet")!;
    expect(met.severe).toBe(true);
    expect(shots.filter((s) => s.firerId === "B1")).toHaveLength(0);
    const request = ask(after, "B1", "triggerMet", cfg, { about: "R1" });
    expect(request.point).toBe("D4");
    expect(request.options.map((o) => o.id)).toEqual(["fire", "closer", "letPass"]);
    expect(request.fallback).toBe("fire");
  });

  it("offers concrete triggers: the range where the hit chance reaches 80%, and its flank", () => {
    const cfg = config();
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2900), { facing: 180 })], true));
    const moving = { ...state, units: { ...state.units, R1: { ...state.units.R1, lastMovedAt: 0 } } };
    const triggers = triggersFor(moving, moving.game.forceElements.B1, moving.game.forceElements.R1, cfg);
    expect(triggers.length).toBeGreaterThan(0);
    for (const trigger of triggers) expect(["hitChance", "range", "flank"]).toContain(trigger.kind);
    const d1 = ask(state, "B1", "sighted", cfg, { about: "R1" });
    expect(d1.point).toBe("D1");
    expect(d1.options[0].id).toBe("keep");
    expect(d1.options.some((o) => o.id.endsWith(":fire") && o.order.kind === "wait" && o.order.autoFire)).toBe(true);
    expect(d1.options.map((o) => o.id)).toEqual(expect.arrayContaining(["engage", "observe"]));
  });
});

// ── The decision points ────────────────────────────────────────────────────

describe("decision points", () => {
  const ev = (kind: RtEvent["kind"], extra: Partial<RtEvent> = {}): RtEvent => ({ time: 0, unitId: "B1", kind, detail: "", severe: false, ...extra });

  it("maps events to decision points, most pressing first, and asks nothing for the rest", () => {
    expect(decisionPointOf([ev("sighted"), ev("underFire", { located: true })])?.point).toBe("D2");
    expect(decisionPointOf([ev("hit", { located: false })])?.point).toBe("D3");
    expect(decisionPointOf([ev("volley"), ev("friendNeedsHelp")])?.point).toBe("D9");
    expect(decisionPointOf([ev("newOrders"), ev("underFire", { located: true })])?.point).toBe("D0");
    expect(decisionPointOf([ev("rallied")])?.point).toBe("D11");
    expect(decisionPointOf([ev("exposed")])).toBeNull();
    expect(decisionPointOf([ev("friendLost"), ev("moraleDrop"), ev("review")])).toBeNull();
    expect(decisionPointOf([ev("targetGone", { info: true })])).toBeNull();
  });

  it("after its first volley (D6), a firer weighs whether it has been found yet", () => {
    const cfg = noHits();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2500))], true));
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const { state: after, events } = runUntil(state, cfg, (_s, e) => e.some((x) => x.kind === "volley"), 60);
    const volley = events.find((e) => e.kind === "volley")!;
    expect(volley.unitId).toBe("B1");
    const request = ask(after, "B1", "volley", cfg, { about: "R1" });
    expect(request.point).toBe("D6");
    expect(request.options[0].id).toBe("keep");
    expect(request.options[0].summary).toMatch(/no visible reaction yet; .* that it has located you/);
    expect(request.options.map((o) => o.id)).toEqual(expect.arrayContaining(["quiet"]));
  });

  it("a friend close by who knows the attacker is offered the chance to help (D9)", () => {
    const cfg = noHits();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(600, 0)), fe("R1", "red", at(0, 1500))], true));
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    state = setOrder(state, "B1", { kind: "hold" }, cfg, { roe: "never" });
    state = setOrder(state, "B2", { kind: "hold" }, cfg, { roe: "never" });
    const { state: after, events } = runUntil(state, cfg, (_s, e) => e.some((x) => x.kind === "friendNeedsHelp"), 60);
    const help = events.find((e) => e.kind === "friendNeedsHelp")!;
    expect(help).toMatchObject({ unitId: "B2", about: "R1" });
    const request = ask({ ...after, units: { ...after.units, B2: { ...after.units.B2, roe: "always" } } }, "B2", "friendNeedsHelp", cfg, { about: "R1" });
    expect(request.options.map((o) => o.id)).toEqual(expect.arrayContaining(["help", "keep"]));
    expect(request.fallback).toBe("help");
  });

  it("never offers an option that crosses a line its orders forbid", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2800))], true));
    const line = { keep: "south" as const, at: at(0, 500), label: "the road" };
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, orders: ordersOf([{ label: "hold", order: { kind: "hold" } }], { boundaries: [line] }) } } };
    const request = ask(state, "B1", "sighted", cfg, { about: "R1" });
    for (const option of request.options) {
      const to = option.order.kind === "move" || option.order.kind === "withdraw" ? option.order.to : null;
      if (to) expect(to.lat).toBeLessThanOrEqual(line.at.lat);
    }
    expect(request.options.some((o) => o.id === "better")).toBe(false);
  });
});

// ── Orders: phases, constraints, chained orders ────────────────────────────

describe("mission orders", () => {
  it("carries out its phases one after another with no decision, then flags that it is out of orders", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    const orders = ordersOf([
      { label: "to the first bound", order: { kind: "move", to: at(0, 200), mode: "march" } },
      { label: "to the ridge", order: { kind: "move", to: at(200, 400), mode: "march" } },
    ]);
    state = applyOrders(state, { side: "blue", orders: { B1: orders }, plan: "test", warnings: [], by: "claude" }, cfg);
    expect(state.units.B1.order).toMatchObject({ kind: "move", phase: 0 });
    const decider = recording();
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(400);
    const kinds = runner.log.filter((e) => e.type === "event").map((e) => (e.type === "event" ? e.event.kind : ""));
    expect(kinds).toContain("phaseDone");
    expect(kinds).toContain("outOfOrders");
    expect(runner.state.units.B1.orders?.phase).toBe(1);
    expect(distanceM(runner.state.game.forceElements.B1.position, at(200, 400))).toBeLessThan(5);
    expect(runner.flags.map((f) => f.kind)).toContain("outOfOrders");
    // Asked once it had nothing left to do (D10), not at the first phase's end.
    expect(decider.asked.map((r) => r.point)).toEqual(["D10"]);
  });

  it("does not cross a line its orders forbid, whatever the order says", () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    const line = { keep: "south" as const, at: at(0, 300), label: "the road" };
    state = applyOrders(
      state,
      { side: "blue", orders: { B1: ordersOf([{ label: "north", order: { kind: "move", to: at(0, 1000), mode: "march" } }], { boundaries: [line] }) }, plan: "", warnings: [], by: "claude" },
      cfg,
    );
    const { state: after, events } = runUntil(state, cfg, () => false, 200);
    expect(events.find((e) => e.kind === "blocked")?.detail).toMatch(/stopped short of the road/);
    expect(after.game.forceElements.B1.position.lat).toBeLessThanOrEqual(line.at.lat);
  });

  it("fire and move: a set number of volleys, then the next order", () => {
    const cfg = noHits();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2000))], true));
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1", volleys: 1, then: { kind: "withdraw", to: at(0, -500) } }, cfg);
    state = setOrder(state, "R1", { kind: "hold" }, cfg, { roe: "never" });
    const { state: after, shots } = runUntil(state, cfg, (s) => s.units.B1.order.kind === "withdraw", 60);
    expect(shots.filter((s) => s.firerId === "B1")).toHaveLength(1);
    expect(after.units.B1.order).toMatchObject({ kind: "withdraw" });
  });
});

describe("orders it cannot carry out", () => {
  it("is not sent back into the same obstacle: blocked once, flagged, and left for new orders", async () => {
    // Water from 300 m north: the ordered destination is across it.
    const river = { groundHeightM: () => 0, classify: (p: LatLng) => (p.lat > at(0, 300).lat && p.lat < at(0, 600).lat ? "water" : "open") } as const;
    // Impassable, as the raster says a river is (the land cover alone would let it ford).
    const cfg = config({ terrain: river as never, isPassable: (p) => river.classify(p) !== "water" });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = applyOrders(
      state,
      { side: "blue", orders: { B1: ordersOf([{ label: "cross the river", order: { kind: "move", to: at(0, 1000), mode: "tactical" } }]) }, plan: "", warnings: [], by: "claude" },
      cfg,
    );
    const decider = recording();
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(600);
    const kinds = runner.log.flatMap((e) => (e.type === "event" && e.event.unitId === "B1" ? [e.event.kind] : []));
    expect(kinds.filter((k) => k === "blocked")).toHaveLength(1);
    expect(kinds).not.toContain("idle");
    expect(runner.state.units.B1.orders?.blocked?.why).toMatch(/water/);
    expect(runner.flags.some((f) => f.unitId === "B1" && /cannot carry out its orders/.test(f.text))).toBe(true);
    // Asked once (D10), and not offered the phase that cannot be done.
    expect(decider.asked).toHaveLength(1);
    expect(decider.asked[0].options.map((o) => o.id)).not.toContain("resume");
  });
});

describe("a unit carrying out its orders is left alone", () => {
  it("is not called idle on a stationary step, wherever its mission was worked out from", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = applyOrders(
      state,
      { side: "blue", orders: { B1: ordersOf([{ label: "observe the eastern approaches", order: { kind: "observe" } }]) }, plan: "", warnings: [], by: "claude" },
      cfg,
    );
    // As after a skipped step: the mission's place is somewhere it never got to.
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, mission: { task: "hold", at: at(2000, 2000), purpose: "reserve" } } } };
    const decider = recording();
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(600);
    expect(runner.log.some((e) => e.type === "event" && e.event.kind === "idle")).toBe(false);
    expect(decider.asked).toHaveLength(0);
  });
});

describe("a unit standing on ground it cannot move on", () => {
  it("gets out to the nearest ground it can move on, then follows its route", () => {
    const water = (p: LatLng) => p.lat > at(0, 200).lat && p.lat < at(0, 260).lat;
    const cfg = config({ isPassable: (p) => !water(p) });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 230)), fe("R1", "red", at(0, 30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, -500), mode: "tactical" }, cfg);
    const { state: after, events } = runUntil(state, cfg, (s) => s.units.B1.order.kind !== "move", 900);
    expect(events.filter((e) => e.kind === "blocked")).toHaveLength(0);
    expect(distanceM(after.game.forceElements.B1.position, at(0, -500))).toBeLessThan(5);
  });

  it("says why when it is blocked, so a stuck unit can be diagnosed", () => {
    const water = (p: LatLng) => p.lat > at(0, 200).lat && p.lat < at(0, 260).lat;
    const cfg = config({ isPassable: (p) => !water(p), planner: { kind: "raster", plan: () => null } });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    state = setOrder(state, "B1", { kind: "move", to: at(0, 1000), mode: "tactical" }, cfg);
    const { events } = runUntil(state, cfg, (_s, e) => e.some((x) => x.kind === "blocked"), 900);
    expect(events.find((e) => e.kind === "blocked")?.detail).toMatch(/no route could be planned from here/);
  });
});

describe("new orders while the clock runs (D0)", () => {
  const fight = () => {
    const cfg = noHits();
    let state = createRealtimeState(
      // B1's crew has the nerve not to be pinned into holding: the test is about its orders.
      game([fe("B1", "blue", at(0, 0), { troopQuality: 12 }), fe("B2", "blue", at(2000, -3000)), fe("R1", "red", at(0, 2000)), fe("R9", "red", at(9000, 30_000))], true),
    );
    state = setOrder(state, "B1", { kind: "engage", targetId: "R1" }, cfg);
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    return { cfg, state };
  };
  const newOrders = (urgency: UnitOrders["urgency"]) =>
    ordersOf([{ label: "fall back to the woods", order: { kind: "move", to: at(0, -2000), mode: "tactical" } }], { urgency });

  it("a unit out of contact takes its new orders at once; one in a fight is asked how to comply", async () => {
    const { cfg, state } = fight();
    const decider = recording();
    const runner = new RealtimeRunner(state, cfg, { deciders: { blue: decider } });
    await runner.advance(40);
    runner.issueOrders({ B1: newOrders("whenAble"), B2: newOrders("whenAble") }, { side: "blue", text: "fall back" });
    // B2 was not in contact: it is already on its way, no call.
    expect(runner.state.units.B2.order).toMatchObject({ kind: "move", phase: 0 });
    expect(runner.awaitingOrders).toEqual(["B1"]);
    await runner.advance(30);
    const d0 = decider.asked.find((r) => r.unitId === "B1" && r.point === "D0")!;
    // (Breaking contact needs somewhere out of sight to go; this ground is flat.)
    expect(d0.options.map((o) => o.id)).toEqual(["comply", "finish"]);
    expect(d0.options.every((o) => o.orders === d0.options[0].orders)).toBe(true);
    // The rules finish the fight first — time-boxed.
    expect(d0.fallback).toBe("finish");
    expect(runner.state.units.B1.orders?.task).toBe("fall back to the woods");
    // Whatever it decides meanwhile, the fight stays time-boxed.
    const order = runner.state.units.B1.order;
    expect(order).toMatchObject({ kind: "engage", targetId: "R1", then: { kind: "move", phase: 0 } });
    // …and the order runs, with no further call, when the time is up (asked at 40 s, so at 160 s).
    await runner.advance(160 - runner.state.time);
    expect(runner.state.units.B1.order).toMatchObject({ kind: "move", phase: 0 });
    expect(runner.state.plan.blue).toBe("fall back");
  });

  it("an order marked 'now' breaks off rather than finishing the fight", () => {
    const { cfg, state: start } = fight();
    let state = { ...start, time: 30 };
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, attackers: { R1: 25 }, lastIncomingAt: 25 } } };
    const request = ask(state, "B1", "newOrders", cfg, {}, { orders: newOrders("now") });
    expect(request.point).toBe("D0");
    expect(request.options[0].id).toBe("comply");
    expect(request.options.map((o) => o.id)).not.toContain("finish");
    expect(request.fallback).toBe("comply");
  });
});

// ── The commander's orders ─────────────────────────────────────────────────

describe("the commander's orders", () => {
  const base = () => {
    const state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(500, 0)), fe("R1", "red", at(0, 2000)), fe("R2", "red", at(0, 9000))]),
    );
    // Blue has sighted R1 only: B1 saw it a minute ago.
    return {
      ...state,
      time: 60,
      game: { ...state.game, sighting: { blue: { R1: "full" as const }, red: {} } },
      units: { ...state.units, B1: { ...state.units.B1, ownSeen: { R1: { time: 0, level: "full" as const, at: at(0, 2000), moving: true } } } },
    };
  };

  it("builds a prompt from what the side knows, with reference points and the player's guidance", () => {
    const cfg = config();
    const prompt = ordersPrompt(base(), "blue", cfg, { guidance: "hold the bridge" });
    expect(prompt).toMatch(/Reference points: OBJECTIVE, B1, B2, R1/);
    expect(prompt).toMatch(/The player's guidance, which comes first: hold the bridge/);
    expect(prompt).not.toMatch(/R2/);
    // HQ's picture: when it was seen, by whom, and how far it may have moved.
    expect(prompt).toMatch(/"seen": "1:00 ago, by B1"/);
    expect(prompt).toMatch(/"mayHaveMoved": "up to \d+ m since"/);
  });

  it("reads orders, checks every field, and says what it dropped", () => {
    const state = base();
    const reply = `Here you are:
{"plan": "B1 fixes, B2 flanks",
 "orders": [
  {"unit": "B1", "task": "hold and fix R1", "intent": "keep R1 busy", "urgency": "now", "roe": "always", "onContact": "engage",
   "phases": [{"label": "hull-down west", "do": "move", "to": {"ref": "B1", "bearingDeg": 270, "distanceM": 300}, "mode": "tactical"},
              {"label": "overwatch R1", "do": "overwatch"},
              {"label": "never reached", "do": "hold"}],
   "constraints": [{"stay": "south", "of": {"ref": "OBJECTIVE", "bearingDeg": 180, "distanceM": 3000}, "label": "the road"},
                   {"stay": "north", "of": {"ref": "OBJECTIVE"}, "label": "wrong side"},
                   {"stay": "west", "of": "B2", "label": "behind B2"}]},
  {"unit": "B2", "phases": [{"do": "teleport"}, {"label": "flank", "do": "move", "to": "NOWHERE"}]},
  {"unit": "R1", "phases": [{"do": "hold"}]}
 ]}`;
    const result = parseOrders(reply, state, "blue");
    expect(result.plan).toBe("B1 fixes, B2 flanks");
    const b1 = result.orders.B1;
    expect(b1).toMatchObject({ urgency: "now", roe: "always", onContact: "engage", intent: "keep R1 busy", by: "claude" });
    expect(b1.phases.map((p) => p.order.kind)).toEqual(["move", "overwatch"]);
    const to = (b1.phases[0].order as { to: LatLng }).to;
    expect(distanceM(to, at(-300, 0))).toBeLessThan(5);
    expect(b1.boundaries).toHaveLength(1);
    expect(result.orders.B2).toBeUndefined();
    expect(result.orders.R1).toBeUndefined();
    expect(result.warnings.join("\n")).toMatch(/phases after "overwatch R1" dropped/);
    expect(result.warnings.join("\n")).toMatch(/already on the other side/);
    expect(result.warnings.join("\n")).toMatch(/placed on B2 was dropped: lines are fixed on the ground/);
    expect(result.warnings.join("\n")).toMatch(/unknown step "teleport"/);
    expect(result.warnings.join("\n")).toMatch(/"R1": not one of your units/);
  });

  it("falls back to the heuristic's orders when the commander cannot be reached or read", async () => {
    const cfg = config();
    const down = await commanderOrders(async () => { throw new Error("offline"); }, base(), "blue", cfg);
    expect(down.by).toBe("heuristic");
    expect(down.warnings[0]).toMatch(/offline/);
    const garbage = await commanderOrders(async () => "I would advance boldly.", base(), "blue", cfg);
    expect(garbage.by).toBe("heuristic");
    expect(Object.keys(garbage.orders)).toEqual(Object.keys(heuristicOrders(base(), "blue").orders));
  });

  it("gives each unit its orders, rules of engagement and a mission to match", () => {
    const cfg = config();
    const state = applyOrders(base(), heuristicOrders(base(), "blue"), cfg);
    expect(state.units.B1.orders?.intent).toBe("take and hold the objective");
    expect(state.units.B1.roe).toBe("withinShortRange");
    expect(state.units.B1.mission.task).toBe("take");
    expect(state.plan.blue).toMatch(/advance/);
  });
});

describe("flags for the player", () => {
  it("raises heavy losses once, and never calls the commander", async () => {
    const cfg = config();
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(500, 0)), fe("R1", "red", at(0, 30_000))]));
    state = { ...state, game: { ...state.game, forceElements: { ...state.game.forceElements, B1: { ...state.game.forceElements.B1, combatStrength: 2 } } } };
    const runner = new RealtimeRunner(state, cfg);
    await runner.advance(5);
    await runner.advance(5);
    expect(runner.flags.filter((f) => f.kind === "losses")).toHaveLength(1);
    expect(runner.flags[0].text).toMatch(/blue has lost 25%/);
  });
});

describe("win conditions", () => {
  const twoEach = () => {
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(500, 0)), fe("R1", "red", at(0, 30_000)), fe("R2", "red", at(500, 30_000))]),
    );
    // Half of blue has broken.
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, cohesion: "broken" } } };
    return state;
  };

  it("ends at the breakpoint set for the game, not a fixed half", () => {
    expect(tick(twoEach(), config()).state.over?.winner).toBe("red");
    const lenient = config({ timing: { ...DEFAULT_TIMING, breakpoint: 0.75 } });
    expect(tick(twoEach(), lenient).state.over).toBeUndefined();
  });

  it("ends at the time limit set for the game", () => {
    const cfg = config({ timing: { ...DEFAULT_TIMING, maxDurationS: 60 } });
    const state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 30_000))]));
    const { state: after } = runUntil(state, cfg, (s) => s.over != null, 200);
    expect(after.time).toBe(60);
    expect(after.over?.reason).toBe("time limit");
  });
});
