/**
 * The radio net (comms.ts): how reports travel, how fast, and to whom; and
 * what each unit knows as a result. docs/REALTIME_COMMS.md.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { metresPerDegreeLon, type LatLng } from "../../lib/board";
import { flatTerrain } from "../../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../../lib/state";
import { createRng } from "../../rules/dice";
import { HOUSE_V1 } from "../../rules/ruleset";
import { COMPOSE_S, RELAY_S, TRANSMIT_S, trackErrorM } from "./comms";
import { detectionRate } from "./detection";
import { createRealtimeState, setOrder, tick, type TickResult } from "./engine";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig, RtEvent, RtState } from "./types";

beforeEach(() => {
  for (const k of ["groupCollapsed", "groupEnd", "log", "info", "table", "warn"] as const) {
    vi.spyOn(console, k).mockImplementation(() => {});
  }
});

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
const at = (east: number, north: number): LatLng => ({
  lat: ORIGIN.lat + north / 111_320,
  lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat),
});

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

function game(elements: ForceElement[]): GameState {
  return {
    gameId: "comms",
    scenarioId: "comms",
    turn: 1,
    phase: "arcAction",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((e) => [e.id, e])),
    sighting: { blue: {}, red: {} },
    objectives: { blue: at(0, 5000), red: at(0, -5000) },
    rng: { seed: "comms", cursor: 0 },
  };
}

const radio = (extra: Partial<RtConfig> = {}): RtConfig => ({
  ruleset: HOUSE_V1,
  terrain: flatTerrain(),
  rng: createRng("comms"),
  timing: { ...DEFAULT_TIMING, strikeScale: 0 },
  comms: "radio",
  ...extra,
});

/** Nobody fires or moves: the tests are about who hears what, when. */
function quiet(state: RtState, cfg: RtConfig): RtState {
  for (const id of Object.keys(state.units)) state = setOrder(state, id, { kind: "hold" }, cfg, { roe: "never" });
  return state;
}

function run(state: RtState, cfg: RtConfig, seconds: number) {
  const results: TickResult[] = [];
  for (let i = 0; i < seconds; i += 1) {
    const r = tick(state, cfg);
    results.push(r);
    state = r.state;
  }
  return { state, results, events: results.flatMap((r) => r.events), messages: results.flatMap((r) => r.messages) };
}

// B1 sees R1 at point-blank (no roll). HQ1 and B2 are over 3 km from R1 (out of sight) but on the net.
const layout = (hq = true) =>
  game([
    fe("B1", "blue", at(0, 0)),
    ...(hq ? [fe("HQ1", "blue", at(0, -3200), { commandRating: 3 })] : []),
    fe("B2", "blue", at(0, -4500)),
    fe("R1", "red", at(0, 140)),
  ]);

describe("reports on the radio net", () => {
  it("go to the HQ first, and from it to everyone else: two hops", () => {
    const cfg = radio();
    const { state, messages } = run(quiet(createRealtimeState(layout()), cfg), cfg, 60);
    const contact = messages.filter((m) => m.message.kind === "contact" && m.message.contact?.enemyId === "R1");
    expect(contact[0]).toMatchObject({ to: ["HQ1"], message: { from: "B1", sender: "B1", hop: 1 } });
    expect(contact[1]).toMatchObject({ message: { from: "B1", sender: "HQ1", hop: 2 } });
    expect(contact[1].to).toContain("B2");
    // Composed, sent, passed on and sent again.
    expect(contact[0].time - 1).toBeGreaterThanOrEqual(COMPOSE_S + TRANSMIT_S);
    expect(contact[1].time - contact[0].time).toBeGreaterThanOrEqual(RELAY_S + TRANSMIT_S);
    expect(state.units.B2.picture.R1).toMatchObject({ from: "B1", via: "HQ1", level: "full" });
  });

  it("with no HQ, go straight to whoever is in range, a little slower to compose", () => {
    const cfg = radio();
    const { messages } = run(quiet(createRealtimeState(layout(false)), cfg), cfg, 60);
    const contact = messages.find((m) => m.message.kind === "contact")!;
    expect(contact.to).toEqual(["B2"]);
    expect(contact.time - 1).toBeGreaterThanOrEqual(COMPOSE_S * 1.5 + TRANSMIT_S);
  });

  it("do not reach a unit out of radio range", () => {
    const cfg = radio();
    const far = game([fe("B1", "blue", at(0, 0)), fe("B9", "blue", at(0, -9000)), fe("R1", "red", at(0, 140))]);
    const { state } = run(quiet(createRealtimeState(far), cfg), cfg, 120);
    expect(state.units.B9.picture.R1).toBeUndefined();
  });

  it("with perfect comms, reach every friend after the report delay, as before", () => {
    const cfg = radio({ comms: "perfect" });
    const { state, messages } = run(quiet(createRealtimeState(layout()), cfg), cfg, 30);
    const contact = messages.find((m) => m.message.kind === "contact")!;
    expect(contact.time - 1).toBe(DEFAULT_TIMING.reportDelayS);
    expect(contact.to.sort()).toEqual(["B2", "HQ1"]);
    expect(state.units.B2.picture.R1).toBeDefined();
  });

  it("share one net: one talker at a time", () => {
    const cfg = radio();
    // Four troops each spot their own enemy at the same moment.
    const g = game([
      ...[0, 1, 2, 3].map((i) => fe(`B${i}`, "blue", at(i * 800, 0))),
      ...[0, 1, 2, 3].map((i) => fe(`R${i}`, "red", at(i * 800, 140))),
    ]);
    const { messages } = run(quiet(createRealtimeState(g), cfg), cfg, 120);
    const onAir = messages.filter((m) => m.message.side === "blue").map((m) => m.time);
    for (let i = 1; i < onAir.length; i += 1) expect(onAir[i] - onAir[i - 1]).toBeGreaterThanOrEqual(TRANSMIT_S);
  });

  it("a report goes stale: how far off it may be grows with its age", () => {
    const track = { level: "full" as const, at: at(0, 0), seenAt: 0, receivedAt: 20, from: "B1", errorM: 50, moving: true };
    expect(trackErrorM(track, 0)).toBe(50);
    expect(trackErrorM(track, 60)).toBeGreaterThan(300);
    expect(trackErrorM({ ...track, moving: false }, 60)).toBeLessThan(100);
  });

  it("a reported enemy is not fired on until the unit finds it — though knowing where to look helps", () => {
    expect(detectionRate(2000, { observerCued: true })).toBeCloseTo(detectionRate(2000, {}) * 3, 6);
    const cfg = radio({ comms: "perfect", timing: { ...DEFAULT_TIMING, strikeScale: 0 } });
    let state = createRealtimeState(game([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 2500))]));
    state = quiet(state, cfg);
    // B1 has been told where R1 is, but has not seen it.
    state = {
      ...state,
      units: {
        ...state.units,
        B1: {
          ...state.units.B1,
          roe: "always",
          order: { kind: "overwatch" },
          picture: { R1: { level: "full", at: at(0, 2500), seenAt: 0, receivedAt: 0, from: "B9", errorM: 50, moving: false } },
        },
      },
    };
    const first = tick(state, cfg);
    expect(first.shots.filter((s) => s.firerId === "B1")).toHaveLength(0);
    expect(first.state.units.B1.ownSeen.R1).toBeUndefined();
  });

  it("a pinned crew sends only 'under fire' and calls for help: no contact reports", () => {
    const cfg = radio();
    let state = quiet(createRealtimeState(layout(false)), cfg);
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, suppression: 90, lastIncomingAt: 0 } } };
    const { messages } = run(state, cfg, 60);
    expect(messages.filter((m) => m.message.from === "B1" && m.message.kind === "contact")).toHaveLength(0);
  });
});

describe("a friend in trouble, heard on the net", () => {
  it("raises D9 when the 'under fire' report arrives, not the moment the shot is fired", () => {
    const cfg = radio({ timing: { ...DEFAULT_TIMING, strikeScale: 0 } });
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(600, 0)), fe("R1", "red", at(0, 1500))]),
    );
    // Both blue troops know R1; R1 fires on B1.
    state = { ...state, game: { ...state.game, sighting: { blue: { R1: "full" }, red: { B1: "full", B2: "full" } } } };
    state = createRealtimeState(state.game);
    state = quiet(state, cfg);
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    const { events } = run(state, cfg, 90);
    const fired = events.find((e: RtEvent) => e.unitId === "B1" && e.kind === "underFire")!;
    const help = events.find((e: RtEvent) => e.unitId === "B2" && e.kind === "friendNeedsHelp")!;
    expect(help).toMatchObject({ about: "R1" });
    expect(help.time - fired.time).toBeGreaterThanOrEqual(COMPOSE_S + TRANSMIT_S);
  });
});

describe("asking friends for help", () => {
  it("a request goes over the net, the friend is asked (D12), complies, and the reply comes back", async () => {
    const { RealtimeRunner } = await import("./runner");
    const { helperFor } = await import("./decisions");
    const cfg = radio({ comms: "perfect", timing: { ...DEFAULT_TIMING, strikeScale: 0 } });
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(600, 0)), fe("R1", "red", at(0, 1500))]),
    );
    state = { ...state, game: { ...state.game, sighting: { blue: { R1: "full" }, red: { B1: "full" } } } };
    state = quiet(createRealtimeState(state.game), cfg);
    // B1 and B2 may fire; R1 opens fire on B1.
    for (const id of ["B1", "B2"]) state = setOrder(state, id, { kind: "hold" }, cfg, { roe: "withinShortRange" });
    state = setOrder(state, "R1", { kind: "engage", targetId: "B1" }, cfg);
    expect(helperFor(state, "B1", "R1")).toBe("B2");
    const asked: { unit: string; point: string; ids: string[] }[] = [];
    const runner = new RealtimeRunner(state, cfg, {
      deciders: {
        blue: {
          name: "script",
          async decide(_s, _side, requests) {
            return requests.map((r) => {
              asked.push({ unit: r.unitId, point: r.point, ids: r.options.map((o) => o.id) });
              const pick = r.point === "D2" && r.options.some((o) => o.id === "callFire") ? "callFire" : r.point === "D12" ? "comply" : r.fallback;
              return { unitId: r.unitId, optionId: r.options.some((o) => o.id === pick) ? pick : r.fallback, trace: { question: `${r.point}: x`, options: [], chosenId: pick, chosenBy: "jev" as const } };
            });
          },
        },
      },
    });
    await runner.advance(180);
    expect(asked.find((a) => a.unit === "B1" && a.point === "D2")?.ids).toContain("callFire");
    const d12 = asked.find((a) => a.unit === "B2" && a.point === "D12")!;
    expect(d12.ids).toEqual(expect.arrayContaining(["comply", "keep"]));
    expect(runner.state.units.B2.order).toMatchObject({ kind: "engage", targetId: "R1" });
    expect(runner.state.units.B2.requests).toHaveLength(0);
    expect(runner.state.units.B1.heard.some((h) => h.kind === "reply" && /complying/.test(h.text))).toBe(true);
    const kinds = runner.log.flatMap((e) => (e.type === "message" ? [e.delivery.message.kind] : []));
    expect(kinds).toEqual(expect.arrayContaining(["request", "reply"]));
  });

  it("asks the friend its orders say supports it, before the nearest one", async () => {
    const { helperFor } = await import("./decisions");
    let state = createRealtimeState(
      game([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(300, 0)), fe("B3", "blue", at(2500, 0)), fe("R1", "red", at(0, 9000))]),
    );
    expect(helperFor(state, "B1")).toBe("B2");
    const orders = {
      task: "support B1",
      phases: [{ label: "overwatch", order: { kind: "overwatch" as const } }],
      phase: 0,
      intent: "",
      urgency: "whenAble" as const,
      roe: "withinShortRange" as const,
      onContact: "engage" as const,
      boundaries: [],
      by: "claude" as const,
      issuedAt: 0,
      supports: "B1",
    };
    state = { ...state, units: { ...state.units, B3: { ...state.units.B3, orders } } };
    expect(helperFor(state, "B1")).toBe("B3");
  });
});
