// ── bgws/realtime/engine/situations.ts ─────────────────────────────────────
// A hand-checked situation for every decision point: the calibration set.
//
// Each one is a small, legible moment — a hidden troop spotting a column far
// off, a troop fired on by a shooter it cannot see — with the options a
// sensible leader might take, judged by hand, and what the rules take.
//
// Two uses (docs/REALTIME_COMMAND_DESIGN.html §9, "tuning"):
//
//   · situations.test.ts checks the rules' fallback is sensible in every one.
//   · With an OpenRouter key, the same file sends each to Jev and reports,
//     for each confidence threshold, how many of Jev's answers it would take
//     and how many of those are sensible. The threshold is set from that
//     report, on this game's own situations, not guessed.
//
// Everything is on small made-up ground: flat, with a belt of woods behind
// the blue troop where a hidden spot or cover is needed.

import { metresPerDegreeLon, type LatLng } from "../../lib/board";
import type { TerrainSampler } from "../../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../../lib/state";
import { createRng } from "../../rules/dice";
import { HOUSE_V1 } from "../../rules/ruleset";
import type { RtDecisionRequest } from "./deciders";
import { decisionPointOf, optionsAt, ruleFallback, type DecisionContext, type DecisionPoint } from "./decisions";
import { createRealtimeState, setOrder } from "./engine";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig, RtEvent, RtOrder, RtState, RtUnit, UnitOrders } from "./types";

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
/** Metres east and north of the origin. */
export function at(east: number, north: number): LatLng {
  return { lat: ORIGIN.lat + north / 111_320, lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat) };
}

/** Flat, with a belt of woods 150–300 m south of the blue start line. */
export const WOODS_BEHIND: TerrainSampler = {
  groundHeightM: () => 0,
  classify: (p) => (p.lat < at(0, -150).lat && p.lat > at(0, -300).lat ? "woodsLight" : "open"),
};

function troop(id: string, side: Side, position: LatLng, extra: Partial<ForceElement> = {}): ForceElement {
  return {
    id,
    side,
    label: side === "blue" ? `Challenger troop ${id}` : `T-90 troop ${id}`,
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

function board(elements: ForceElement[]): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  for (const e of elements) sighting[e.side === "blue" ? "red" : "blue"][e.id] = "full";
  return {
    gameId: "situations",
    scenarioId: "situations",
    turn: 1,
    phase: "arcAction",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((e) => [e.id, e])),
    sighting,
    objectives: { blue: at(0, 4000), red: at(0, -4000) },
    rng: { seed: "situations", cursor: 0 },
  };
}

function orders(phases: UnitOrders["phases"], extra: Partial<UnitOrders> = {}): UnitOrders {
  return {
    task: phases.map((p) => p.label).join(", then "),
    phases,
    phase: 0,
    intent: "hold this ground so the rest of the squadron can cross behind us",
    urgency: "whenAble",
    roe: "withinShortRange",
    onContact: "engage",
    boundaries: [],
    by: "claude",
    issuedAt: 0,
    ...extra,
  };
}

const HOLD = orders([{ label: "hold the crossing", order: { kind: "overwatch" } }]);

export interface Situation {
  id: string;
  point: DecisionPoint;
  /** What is going on, in a sentence. */
  story: string;
  /** Options a sensible leader might take, judged by hand. */
  sensible: string[];
  build(): { state: RtState; config: RtConfig; request: RtDecisionRequest };
}

interface Scene {
  blue?: Partial<ForceElement>;
  red?: { at: LatLng; extra?: Partial<ForceElement> }[];
  friends?: LatLng[];
  unit?: Partial<RtUnit>;
  order?: RtOrder;
  event: Partial<RtEvent> & Pick<RtEvent, "kind">;
  context?: DecisionContext;
  time?: number;
  prepare?: (state: RtState) => RtState;
}

function scene(s: Scene) {
  return () => {
    const config: RtConfig = { ruleset: HOUSE_V1, terrain: WOODS_BEHIND, rng: createRng("situations"), timing: DEFAULT_TIMING };
    const reds = (s.red ?? []).map((r, i) => troop(`R${i + 1}`, "red", r.at, r.extra));
    const friends = (s.friends ?? []).map((p, i) => troop(`B${i + 2}`, "blue", p));
    let state = createRealtimeState(board([troop("B1", "blue", at(0, 0), s.blue), ...friends, ...reds]));
    state = { ...state, time: s.time ?? 60 };
    state = setOrder(state, "B1", s.order ?? { kind: "overwatch", phase: 0 }, config);
    state = { ...state, units: { ...state.units, B1: { ...state.units.B1, orders: HOLD, posture: "settled", ...s.unit } } };
    for (const red of reds) state = setOrder(state, red.id, { kind: "hold" }, config, { roe: "never" });
    if (s.prepare) state = s.prepare(state);
    const event: RtEvent = { time: state.time, unitId: "B1", detail: s.event.kind, severe: false, ...s.event };
    const { point } = decisionPointOf([event])!;
    const options = optionsAt(state, "B1", point, event, config, s.context);
    const request: RtDecisionRequest = {
      unitId: "B1",
      point,
      event,
      events: [event],
      options,
      fallback: ruleFallback(state, "B1", point, options),
    };
    return { state, config, request };
  };
}

/** Fired on by R1 a moment ago, and it knows. */
const firedOnBy = (id: string, time = 58) => (state: RtState): RtState => ({
  ...state,
  units: {
    ...state.units,
    B1: {
      ...state.units.B1,
      attackers: { [id]: time },
      lastIncomingAt: time,
      incoming: { [id]: { shots: 1, damage: 0, since: time, last: time } },
    },
  },
});

export const SITUATIONS: Situation[] = [
  {
    id: "D1-close-good-shot",
    point: "D1",
    story: "B1, on overwatch, finds an enemy troop at 900 m, side-on to it.",
    sensible: ["engage", "wait0:fire", "wait1:fire"],
    build: scene({ red: [{ at: at(0, 900), extra: { facing: 90 } }], event: { kind: "sighted", about: "R1" } }),
  },
  {
    id: "D1-far-unseen",
    point: "D1",
    story: "B1, hidden and unseen, finds an enemy troop at 2.8 km, coming head-on.",
    sensible: ["wait0", "wait0:fire", "wait1", "wait1:fire", "observe", "keep"],
    build: scene({ red: [{ at: at(0, 2800) }], event: { kind: "sighted", about: "R1" } }),
  },
  {
    id: "D1-observe-orders",
    point: "D1",
    story: "B1 is a screen told to observe and report; it finds an enemy troop at 1.5 km.",
    sensible: ["observe", "keep", "wait0", "wait1"],
    build: scene({
      red: [{ at: at(0, 1500) }],
      unit: { orders: { ...HOLD, onContact: "observe", intent: "report the enemy's approach without being seen" } },
      event: { kind: "sighted", about: "R1" },
    }),
  },
  {
    id: "D1-avoid-orders",
    point: "D1",
    story: "B1 is told to avoid a fight; an enemy troop appears at 1.2 km, and B2 is behind it.",
    sensible: ["pullBack", "observe"],
    build: scene({
      red: [{ at: at(0, 1200) }],
      friends: [at(0, -1500)],
      unit: { orders: { ...HOLD, onContact: "avoid", intent: "stay alive to hold the rear crossing later" } },
      event: { kind: "sighted", about: "R1" },
    }),
  },
  {
    id: "D2-open-located",
    point: "D2",
    story: "B1, in the open, is fired on from 1.2 km by a troop it can see.",
    sensible: ["returnFire", "hullDown", "pullBack"],
    build: scene({ red: [{ at: at(0, 1200) }], prepare: firedOnBy("R1"), event: { kind: "underFire", about: "R1", located: true } }),
  },
  {
    id: "D2-avoid",
    point: "D2",
    story: "B1, told to avoid a fight, is fired on from 1.5 km by a troop it can see.",
    sensible: ["pullBack", "quiet"],
    build: scene({
      red: [{ at: at(0, 1500) }],
      unit: { orders: { ...HOLD, onContact: "avoid" } },
      prepare: firedOnBy("R1"),
      event: { kind: "underFire", about: "R1", located: true },
    }),
  },
  {
    id: "D3-open-unseen-shooter",
    point: "D3",
    story: "B1, in the open, is fired on from somewhere to the north; it cannot see the shooter. Woods are just behind it.",
    sensible: ["cover", "search", "pullBack"],
    build: scene({ red: [{ at: at(0, 2600) }], event: { kind: "underFire", about: "R1", located: false, bearingDeg: 0 } }),
  },
  {
    id: "D3-in-cover",
    point: "D3",
    story: "B1, in the woods, is fired on from the north by a shooter it cannot see.",
    sensible: ["search", "keep", "pullBack"],
    build: scene({
      blue: { position: at(0, -200) },
      red: [{ at: at(0, 2600) }],
      event: { kind: "underFire", about: "R1", located: false, bearingDeg: 0 },
    }),
  },
  {
    id: "D4-trigger-met",
    point: "D4",
    story: "B1 has waited, hidden, for the enemy troop to come inside 800 m. It has.",
    sensible: ["fire"],
    build: scene({
      red: [{ at: at(0, 780) }],
      order: { kind: "wait", targetId: "R1", trigger: { kind: "range", withinM: 800 }, autoFire: false, met: true },
      event: { kind: "triggerMet", about: "R1", severe: true },
    }),
  },
  {
    id: "D5-halted",
    point: "D5",
    story: "B1 is waiting in ambush; the enemy troop it is watching at 1.6 km has just halted.",
    sensible: ["keep", "relocate", "fireFirst"],
    build: scene({
      red: [{ at: at(0, 1600) }],
      order: { kind: "wait", targetId: "R1", trigger: { kind: "range", withinM: 800 }, autoFire: true },
      unit: { lastCue: { time: 60, enemyId: "R1", cue: "halted" } },
      event: { kind: "cue", about: "R1", severe: true },
    }),
  },
  {
    id: "D6-first-volley",
    point: "D6",
    story: "B1 has fired its first volley at an enemy troop 1.8 km off; no visible reaction yet.",
    sensible: ["keep", "fireAndMove"],
    build: scene({
      red: [{ at: at(0, 1800) }],
      order: { kind: "engage", targetId: "R1" },
      unit: {
        lastShotAt: 58,
        engagement: { targetId: "R1", since: 58, lastShotAt: 58, shots: 1, hits: 0, damage: 0, window: { since: 58, shots: 1, hits: 0, damage: 0 } },
      },
      event: { kind: "volley", about: "R1" },
    }),
  },
  {
    id: "D7-target-lost",
    point: "D7",
    story: "The troop B1 was engaging has dropped out of sight; B1's orders are to hold the crossing.",
    sensible: ["resume", "watch"],
    build: scene({
      red: [{ at: at(0, 2000) }],
      order: { kind: "hold" },
      prepare: (state) => ({
        ...state,
        game: { ...state.game, sighting: { ...state.game.sighting, blue: {} } },
        lastKnown: { ...state.lastKnown, blue: { R1: { at: at(0, 2000), time: 40 } } },
      }),
      event: { kind: "targetGone", about: "R1" },
    }),
  },
  {
    id: "D8-long-range-misses",
    point: "D8",
    story: "B1 has been trading fire at 2.8 km for three minutes and knocked nothing out.",
    sensible: ["better", "quiet", "pullBack", "shift"],
    build: scene({
      red: [{ at: at(0, 2800) }],
      order: { kind: "engage", targetId: "R1" },
      time: 240,
      unit: {
        lastShotAt: 238,
        engagement: { targetId: "R1", since: 60, lastShotAt: 238, shots: 18, hits: 3, damage: 0, window: { since: 60, shots: 18, hits: 3, damage: 0 } },
      },
      event: { kind: "ineffective", about: "R1" },
    }),
  },
  {
    id: "D9-friend-hit",
    point: "D9",
    story: "B2, 600 m away, is losing tanks to an enemy troop B1 can see and reach at 1.4 km.",
    sensible: ["help", "support"],
    build: scene({ red: [{ at: at(600, 1400) }], friends: [at(600, 0)], event: { kind: "friendNeedsHelp", about: "R1", severe: true } }),
  },
  {
    id: "D10-arrived-off-task",
    point: "D10",
    story: "B1 has reached the hull-down spot it moved to; its orders are to hold the crossing, 1 km back.",
    sensible: ["resume"],
    build: scene({
      blue: { position: at(0, 1000) },
      order: { kind: "hold" },
      unit: { orders: orders([{ label: "hold the crossing", order: { kind: "move", to: at(0, 0), mode: "tactical" } }]) },
      event: { kind: "arrived" },
    }),
  },
  {
    id: "D10-out-of-orders",
    point: "D10",
    story: "B1 has carried out all its orders and is on its final position.",
    sensible: ["overwatch", "keep"],
    build: scene({ order: { kind: "hold" }, event: { kind: "outOfOrders" } }),
  },
  {
    id: "D11-rallied",
    point: "D11",
    story: "B1 has rallied after being shaken; its orders are to hold the crossing, 800 m back.",
    sensible: ["resume", "hold"],
    build: scene({
      blue: { position: at(0, -800) },
      order: { kind: "hold" },
      unit: { orders: orders([{ label: "hold the crossing", order: { kind: "move", to: at(0, 0), mode: "tactical" } }]) },
      event: { kind: "rallied" },
    }),
  },
  {
    id: "D12-fire-request",
    point: "D12",
    story: "B2, 800 m away, asks over the radio for B1 to engage the troop firing on it; B1 can see and reach it.",
    sensible: ["comply", "partly"],
    build: scene({
      red: [{ at: at(800, 1400) }],
      friends: [at(800, 0)],
      unit: { requests: [{ id: 1, time: 60, from: "B2", kind: "fire", enemyId: "R1", text: "B2 asks B1 to engage R1" }] },
      event: { kind: "request", about: "R1", requestId: 1, severe: true },
    }),
  },
  {
    id: "D12-busy",
    point: "D12",
    story: "B1 is in its own firefight when B2, out of its sight, asks for cover against an enemy B1 knows nothing of.",
    sensible: ["keep", "partly"],
    build: scene({
      red: [{ at: at(0, 1500) }],
      friends: [at(-2500, 0)],
      order: { kind: "engage", targetId: "R1" },
      prepare: firedOnBy("R1"),
      unit: { requests: [{ id: 2, time: 60, from: "B2", kind: "cover", text: "B2 pulling back: asks B1 to cover" }] },
      event: { kind: "request", requestId: 2, severe: true },
    }),
  },
  {
    id: "D0-when-able",
    point: "D0",
    story: "B1 is in a firefight when orders come to fall back; the order says 'when able'.",
    sensible: ["finish", "comply", "breakContact"],
    build: scene({
      red: [{ at: at(0, 1500) }],
      order: { kind: "engage", targetId: "R1" },
      prepare: firedOnBy("R1"),
      event: { kind: "newOrders", severe: true },
      context: { orders: orders([{ label: "fall back behind the woods", order: { kind: "move", to: at(0, -600), mode: "tactical" } }]) },
    }),
  },
  {
    id: "D0-now",
    point: "D0",
    story: "B1 is in a firefight when orders come to fall back NOW: the flank is collapsing.",
    sensible: ["comply", "fireAndBack", "covered"],
    build: scene({
      red: [{ at: at(0, 1500) }],
      order: { kind: "engage", targetId: "R1" },
      prepare: firedOnBy("R1"),
      event: { kind: "newOrders", severe: true },
      context: {
        orders: orders([{ label: "fall back behind the woods", order: { kind: "move", to: at(0, -600), mode: "tactical" } }], {
          urgency: "now",
          intent: "get behind the woods before the flank collapses",
        }),
      },
    }),
  },
];
