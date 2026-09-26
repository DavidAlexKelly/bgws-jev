/**
 * Jev as a decider: the question, the answer, and what happens when there is
 * no answer.
 *
 * No network. The model call is a fake that answers from a function, which
 * is the point of injecting it — every rule about what Jev may and may not
 * decide can be asserted without one.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { distanceM, metresPerDegreeLon, type LatLng } from "../lib/board";
import { flatTerrain } from "../lib/lineOfSight";
import { projectForSide } from "../lib/fogOfWar";
import type { ForceElement, GameState, Side } from "../lib/state";
import { advanceTurn, startGame } from "../lib/liveGame";
import { scenarioFactory } from "../lib/forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import { heuristicCommander, type Commander } from "./commander";
import { MEETING_ENGAGEMENT_V1 } from "./forceList";
import type { DiceRoll, Rng } from "./dice";
import { createRng } from "./dice";
import { EventLog, type DecisionEvent, type ResolutionEvent } from "./events";
import {
  parseJevAnswers,
  type JevAnswer,
  type JevCall,
  type JevQuestion,
  type JevRequest,
} from "./jev";
import { jevCommander, jevOrdersCommander } from "./jevCommander";
import { jevTacticalDecider } from "./jevDecider";
import { fireOdds, reactionState } from "./jevState";
import type { OrdersRequest, StandingOrders } from "./orders";
import { HOUSE_V1, withModules } from "./ruleset";
import { runTrial } from "./commanderTrial";
import { withPersistentCache } from "../data/jevCache";
import { decisionMarksFor } from "../lib/stepVisuals";
import { withJevAssessments } from "./jevAssess";
import { buildOrdersPrompt } from "./llmCommander";
import { recentEvents } from "./jevState";
import { executePlannedTurn, planOrdersTurn } from "./orders";
import type { TurnStep } from "./turnLoop";
import { heuristicOrdersCommander } from "./orders";
import type { TacticalDecider } from "./tactical";
import {
  attemptSightingInterruptLive,
  chooseOptionLive,
  noStandingOrders,
  optionsFor,
  prefetchReactionsFor,
  reactiveFireLive,
  resolveAction,
  resolveMoveLive,
  runGame,
  runReactiveFire,
  type PhaseConfig,
} from "./turnLoop";

// Every decision is printed to the console by default. Useful in a browser,
// noise in a test run — silenced here, and asserted on where it matters.
beforeEach(() => {
  vi.spyOn(console, "groupCollapsed").mockImplementation(() => {});
  vi.spyOn(console, "groupEnd").mockImplementation(() => {});
  vi.spyOn(console, "log").mockImplementation(() => {});
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "table").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

// ── Fixtures ───────────────────────────────────────────────────────────────

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
function at(east: number, north: number): LatLng {
  return {
    lat: ORIGIN.lat + north / 111_320,
    lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat),
  };
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

/** Everyone sees everyone unless told otherwise. */
function board(elements: ForceElement[], seen = true): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  if (seen) {
    for (const element of elements) {
      sighting[element.side === "blue" ? "red" : "blue"][element.id] = "full";
    }
  }
  return {
    gameId: "jev",
    scenarioId: "jev",
    turn: 2,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((one) => [one.id, one])),
    sighting,
    rng: { seed: "jev", cursor: 0 },
  };
}

/** Sixes: every sighting succeeds, so contact is a certainty. */
function sixes(): Rng {
  const roll = (n: number): DiceRoll => ({ dice: Array(n).fill(6), total: 6 * n, cursor: 0 });
  return { seed: "six", cursor: 0, d6: () => roll(1), d66: () => roll(2), int: () => 0, pick: (xs) => xs[0] };
}

const REACTION = withModules(HOUSE_V1, { reactionFire: true });

function config(extra: Partial<PhaseConfig> = {}, ruleset = REACTION): PhaseConfig {
  return {
    ruleset,
    terrain: flatTerrain(),
    rng: createRng("jev"),
    log: new EventLog(),
    maxTurns: 40,
    ...extra,
  };
}

/** A JevCall that answers from a function and remembers what it was asked. */
function fakeJev(
  answer: (questions: Record<string, JevQuestion>, request: JevRequest) => Record<string, JevAnswer>,
): JevCall & { requests: JevRequest[] } {
  const requests: JevRequest[] = [];
  const call = (async (request: JevRequest) => {
    requests.push(request);
    return { answers: answer(request.questions, request), latencyMs: 42 };
  }) as JevCall & { requests: JevRequest[] };
  call.requests = requests;
  return call;
}

const everyNoul = (p: number) => (questions: Record<string, JevQuestion>) =>
  Object.fromEntries(
    Object.keys(questions).map((key) => [key, { type: "noul", noul: p } as JevAnswer]),
  );

/**
 * A Jev that answers every choice question the same way. `choice` may be a
 * key, or a function that picks a key from the criteria it was offered.
 */
function answers(
  choice: string | ((criteria: Record<string, string>) => string),
  confidence = 0.8,
  p = 0.8,
): JevCall & { requests: JevRequest[] } {
  return fakeJev((questions) =>
    Object.fromEntries(
      Object.entries(questions).map(([key, question]): [string, JevAnswer] => {
        if (question.type === "noul") return [key, { type: "noul", noul: p }];
        const picked = typeof choice === "string" ? choice : choice(question.criteria);
        return [key, { type: "choice", choice: picked, probabilities: { [picked]: p }, confidence }];
      }),
    ),
  );
}

const failing: JevCall = async () => {
  throw new Error("proxy said no");
};

const decisions = (log: EventLog) =>
  log.all().filter((event): event is DecisionEvent => event.type === "decision");
const reactions = (log: EventLog) =>
  log
    .all()
    .filter((event): event is ResolutionEvent => event.type === "resolution" && event.phase === "arcReaction");

function standing(side: Side, id: string, engage: "never" | "ifFiredUpon" | "withinShortRange" | "always"): StandingOrders {
  const orders = noStandingOrders();
  orders[side].set(id, { actorId: id, engage });
  return orders;
}

// A mover and a watcher 1 km apart: in range, in sight, beyond short range.
const mover = () => fe("B1", "blue", at(0, 0));
const watcher = () => fe("R1", "red", at(0, 1000));

// ── Reading answers ────────────────────────────────────────────────────────

describe("reading a Decisions API response", () => {
  const questions: Record<string, JevQuestion> = {
    pick: { type: "choice", instructions: "which?", criteria: { a: "A", b: "B" } },
    yes: { type: "noul", instructions: "yes?" },
  };

  it("reads the documented shape: answers keyed by question, tagged by type", () => {
    const answers = parseJevAnswers(
      {
        answers: {
          pick: { type: "choice", choice: "b", probabilities: { a: 0.2, b: 0.8 }, confidence: 0.6 },
          yes: { type: "noul", noul: 0.7 },
        },
      },
      questions,
    );
    expect(answers.pick).toEqual({
      type: "choice",
      choice: "b",
      probabilities: { a: 0.2, b: 0.8 },
      confidence: 0.6,
    });
    expect(answers.yes).toEqual({ type: "noul", noul: 0.7 });
  });

  it("tolerates answers at the top level and a bare boolean for yes/no", () => {
    const answers = parseJevAnswers({ pick: { choice: "a" }, yes: true }, questions);
    expect(answers.pick?.type).toBe("choice");
    expect(answers.yes).toEqual({ type: "noul", noul: 1 });
  });

  it("never accepts an option that was not offered", () => {
    const answers = parseJevAnswers({ answers: { pick: { choice: "z" } } }, questions);
    expect(answers.pick).toBeUndefined();
  });
});

// ── The arithmetic Jev is given ────────────────────────────────────────────

describe("fire odds", () => {
  const firer = fe("R1", "red", at(0, 1000));
  const target = fe("B1", "blue", at(0, 0));

  it("is a distribution: the outcomes add up to one", () => {
    const odds = fireOdds([firer], target, { rangeM: 1000, maxRangeM: 3000 }, HOUSE_V1);
    expect(odds.pNoEffect + odds.pSuppress + odds.pHit).toBeCloseTo(1, 1);
  });

  it("makes a snap shot no better than a deliberate one", () => {
    const deliberate = fireOdds([firer], target, { rangeM: 1000, maxRangeM: 3000 }, REACTION);
    const snap = fireOdds([firer], target, { rangeM: 1000, maxRangeM: 3000, snapShot: true }, REACTION);
    expect(snap.pHit).toBeLessThanOrEqual(deliberate.pHit);
  });

  it("does not touch the game's dice", () => {
    const cfg = config();
    const before = cfg.rng.cursor;
    reactionState({
      state: board([mover(), watcher()]),
      config: cfg,
      turn: 2,
      round: "actionReaction",
      side: "red",
      actorId: "B1",
      wasFiredUpon: false,
      candidates: [{ reactorId: "R1", rangeM: 1000, capability: "atk", engage: "always", ruleSaysReact: true }],
      maxReactors: 1,
    });
    expect(cfg.rng.cursor).toBe(before);
  });

  it("does not tell the reacting side how shaken the enemy is", () => {
    // Morale is private. The odds are worked against the enemy AS SEEN, so a
    // broken mover and a steady one produce the same state.
    const moment = (morale: ForceElement["morale"]) =>
      JSON.stringify(
        reactionState({
          state: board([fe("B1", "blue", at(0, 0), { morale }), watcher()]),
          config: config(),
          turn: 2,
          round: "actionReaction",
          side: "red",
          actorId: "B1",
          wasFiredUpon: false,
          candidates: [{ reactorId: "R1", rangeM: 1000, capability: "atk", engage: "always", ruleSaysReact: true }],
          maxReactors: 1,
        }),
      );
    expect(moment("broken")).toEqual(moment("good"));
  });
});

// ── Reactive fire at the moment ────────────────────────────────────────────

describe("Jev deciding reactive fire", () => {
  it("changes nothing when no decider is configured", async () => {
    const plain = config();
    const live = config();
    runReactiveFire(board([mover(), watcher()]), "B1", "blue", plain, 2, noStandingOrders(), "actionReaction");
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", live, 2, noStandingOrders(), "actionReaction");
    expect(live.log.all()).toEqual(plain.log.all());
  });

  it("holds fire the declared rules would have taken, when Jev says no", async () => {
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: answers("none", 0.8, 0.9) }) } });
    const next = await reactiveFireLive(
      board([mover(), watcher()]),
      "B1",
      "blue",
      cfg,
      2,
      standing("red", "R1", "always"),
      "actionReaction",
    );
    expect(reactions(cfg.log)).toHaveLength(0);
    expect(next.forceElements.R1.markers).not.toContain("fired");
    const [decision] = decisions(cfg.log);
    expect(decision).toMatchObject({ chosenBy: "jev", chosenId: "none", actorId: "R1" });
    expect(decision.probabilities?.none).toBeCloseTo(0.9);
  });

  it("takes a shot the declared rules would have let pass, when Jev says yes", async () => {
    // ifFiredUpon, and the mover is only moving: the rule holds. Jev may not.
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: answers("f0") }) } });
    await reactiveFireLive(
      board([mover(), watcher()]),
      "B1",
      "blue",
      cfg,
      2,
      standing("red", "R1", "ifFiredUpon"),
      "actionReaction",
    );
    expect(reactions(cfg.log)).toHaveLength(1);
  });

  it("never overrules a commander's order to stay silent", async () => {
    const call = fakeJev(everyNoul(1));
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call }) } });
    await reactiveFireLive(
      board([mover(), watcher()]),
      "B1",
      "blue",
      cfg,
      2,
      standing("red", "R1", "never"),
      "actionReaction",
    );
    expect(reactions(cfg.log)).toHaveLength(0);
    expect(call.requests).toHaveLength(0);
  });

  it("cannot create a reactor the rules did not offer", async () => {
    // Already activated in this round: not eligible, whatever Jev thinks.
    const call = fakeJev(everyNoul(1));
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call }) } });
    const busy = fe("R1", "red", at(0, 1000), { markers: ["activated"] });
    await reactiveFireLive(board([mover(), busy]), "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    expect(reactions(cfg.log)).toHaveLength(0);
    expect(call.requests).toHaveLength(0);
  });

  it("asks for a fire PLAN in one question: nobody, each alone, or each pair", async () => {
    // Pick the pair R2+R3 — the only way to get two shots is to be asked
    // about them together, which is the point: a plan, not three votes.
    const call = answers((criteria) =>
      Object.keys(criteria).find((key) => /^R2 .* and R3 /.test(criteria[key]))!,
    );
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call }) } });
    await reactiveFireLive(
      board([mover(), watcher(), fe("R2", "red", at(300, 1000)), fe("R3", "red", at(-300, 1000))]),
      "B1",
      "blue",
      cfg,
      2,
      noStandingOrders(),
      "actionReaction",
    );
    expect(call.requests).toHaveLength(1);
    const [question] = Object.values(call.requests[0].questions);
    // none + 3 alone + 3 pairs (the cap is two reactors).
    expect(question.type === "choice" && Object.keys(question.criteria)).toHaveLength(7);
    expect(reactions(cfg.log).map((event) => event.actorIds[0]).sort()).toEqual(["R2", "R3"]);
  });

  it("falls back to the declared rules when Jev cannot be reached, and says so", async () => {
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: failing }) } });
    await reactiveFireLive(
      board([mover(), watcher()]),
      "B1",
      "blue",
      cfg,
      2,
      standing("red", "R1", "always"),
      "actionReaction",
    );
    expect(reactions(cfg.log)).toHaveLength(1);
    expect(decisions(cfg.log)[0]).toMatchObject({ chosenBy: "heuristic", fallback: "error" });
  });

  it("does not wait forever", async () => {
    const hang: JevCall = () => new Promise(() => {});
    const cfg = config({
      tactical: { red: jevTacticalDecider({ side: "red", call: hang, timeoutMs: 20 }) },
    });
    await reactiveFireLive(
      board([mover(), watcher()]),
      "B1",
      "blue",
      cfg,
      2,
      standing("red", "R1", "always"),
      "actionReaction",
    );
    expect(decisions(cfg.log)[0].fallback).toBe("timeout");
    expect(reactions(cfg.log)).toHaveLength(1);
  });
});

// ── Contact at the moment ──────────────────────────────────────────────────

describe("Jev deciding whether to press on through contact", () => {
  const advance = {
    id: "B1:move",
    kind: "move" as const,
    actorId: "B1",
    destination: at(2500, 0),
    summary: "B1 advances 2.5 km east",
  };
  // Red is 900 m along the axis and not yet seen.
  const unseen = () => board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(900, 0))], false);
  const choose = (choice: "halt" | "press", confidence = 0.8) =>
    fakeJev(() => ({
      decision: {
        type: "choice",
        choice,
        probabilities: { [choice]: 0.9, [choice === "halt" ? "press" : "halt"]: 0.1 },
        confidence,
      },
    }));

  it("changes nothing when no decider is configured", async () => {
    const plain = config({ rng: sixes() }, HOUSE_V1);
    const live = config({ rng: sixes() }, HOUSE_V1);
    const a = resolveAction(unseen(), advance, plain, 2);
    const b = await resolveMoveLive(unseen(), advance, live, 2);
    expect(b).toEqual(a);
    expect(live.log.all()).toEqual(plain.log.all());
  });

  it("presses on to the destination when Jev says so", async () => {
    const cfg = config(
      { rng: sixes(), tactical: { blue: jevTacticalDecider({ side: "blue", call: choose("press") }) } },
      HOUSE_V1,
    );
    const next = await resolveMoveLive(unseen(), advance, cfg, 2);
    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeLessThan(5);
    expect(decisions(cfg.log)[0]).toMatchObject({ chosenBy: "jev", chosenId: "press" });
  });

  it("goes to ground where it made contact when Jev says halt", async () => {
    const cfg = config(
      { rng: sixes(), tactical: { blue: jevTacticalDecider({ side: "blue", call: choose("halt") }) } },
      HOUSE_V1,
    );
    const next = await resolveMoveLive(unseen(), { ...advance, onContact: "press" }, cfg, 2);
    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeGreaterThan(100);
    expect(next.sighting.blue.R1).not.toBeUndefined();
  });

  it("falls back to the commander's preset when Jev is unsure", async () => {
    const cfg = config(
      {
        rng: sixes(),
        tactical: { blue: jevTacticalDecider({ side: "blue", call: choose("halt", 0.05) }) },
      },
      HOUSE_V1,
    );
    const next = await resolveMoveLive(unseen(), { ...advance, onContact: "press" }, cfg, 2);
    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeLessThan(5);
    expect(decisions(cfg.log)[0]).toMatchObject({ fallback: "lowConfidence", chosenId: "press" });
  });
});

// ── Jev as the whole commander ─────────────────────────────────────────────

describe("Jev as an orders commander", () => {
  const state = board([
    fe("B1", "blue", at(0, 0)),
    fe("B2", "blue", at(200, 0)),
    fe("B3", "blue", at(400, 0)),
    fe("R1", "red", at(0, 1500)),
  ]);
  const option = (actorId: string, kind: "move" | "fire", n: number) => ({
    id: `${actorId}:${kind}:${n}`,
    kind,
    actorId,
    summary: `${actorId} ${kind} ${n}`,
    ...(kind === "fire" ? { targetId: "R1" } : { destination: at(n * 100, 500) }),
  });
  const request: OrdersRequest = {
    side: "blue",
    turn: 2,
    view: projectForSide(state, "blue"),
    optionsByElement: {
      B1: [option("B1", "fire", 1), option("B1", "move", 2)],
      B2: [option("B2", "fire", 1), option("B2", "move", 2)],
      B3: [option("B3", "fire", 1), option("B3", "move", 2)],
    },
    activationBudget: 2,
    reserveLimit: 0,
  };

  it("spends command capacity on the elements it is surest about", async () => {
    // e0 = B1 at 0.6, e1 = B2 at 0.9, e2 = B3 at 0.7 — capacity for two.
    const certainty: Record<string, number> = { e0: 0.6, e1: 0.9, e2: 0.7 };
    const call = fakeJev((questions) => {
      const answers: Record<string, JevAnswer> = {};
      for (const key of Object.keys(questions)) {
        const element = key.split("_")[0];
        if (key.endsWith("_order")) {
          answers[key] = { type: "choice", choice: "o0", probabilities: { o0: certainty[element] }, confidence: 0.5 };
        } else if (key.endsWith("_roe")) {
          answers[key] = { type: "choice", choice: "withinShortRange", probabilities: {}, confidence: 0.5 };
        }
      }
      return answers;
    });

    const orders = await jevOrdersCommander({ side: "blue", call }).planTurn(request);
    expect(call.requests).toHaveLength(1);
    expect(orders.intents.map((intent) => intent.actorId)).toEqual(["B2", "B3"]);
    expect(orders.intents[0].optionId).toBe("B2:fire:1");
    expect(orders.standingOrders).toHaveLength(3);
  });

  it("leaves an element uncommitted when Jev says so", async () => {
    const call = fakeJev((questions) =>
      Object.fromEntries(
        Object.keys(questions)
          .filter((key) => key.endsWith("_order"))
          .map((key) => [key, { type: "choice", choice: "uncommitted", probabilities: {}, confidence: 0.9 } as JevAnswer]),
      ),
    );
    const orders = await jevOrdersCommander({ side: "blue", call }).planTurn(request);
    expect(orders.intents).toHaveLength(0);
  });

  it("hands the turn to its fallback when Jev cannot be reached", async () => {
    const fallback = {
      kind: "heuristic" as const,
      name: "stand-in",
      planTurn: async () => ({ side: "blue" as const, intents: [{ actorId: "B1", optionId: "B1:move:2" }] }),
    };
    const orders = await jevOrdersCommander({ side: "blue", call: failing, fallback }).planTurn(request);
    expect(orders.intents).toEqual([{ actorId: "B1", optionId: "B1:move:2" }]);
    expect(orders.failure).toMatch(/unreachable/);
  });
});

describe("Jev as a per-activation commander", () => {
  const view = projectForSide(board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1500))]), "blue");
  const offered = [
    { id: "a", kind: "hold" as const, actorId: "B1", summary: "B1 holds" },
    { id: "b", kind: "fire" as const, actorId: "B1", targetId: "R1", summary: "B1 fires at R1" },
  ];
  const fallback: Commander = {
    kind: "heuristic",
    name: "stand-in",
    decide: async () => ({ optionId: "a" }),
  };

  it("takes the option Jev chose, by its real id", async () => {
    const call = fakeJev(() => ({
      action: { type: "choice", choice: "o1", probabilities: { o1: 0.8 }, confidence: 0.7 },
    }));
    const chosen = await jevCommander({ side: "blue", call, fallback }).decide(view, offered, "what now?");
    expect(chosen.optionId).toBe("b");
  });

  it("asks its fallback when Jev cannot be reached", async () => {
    const chosen = await jevCommander({ side: "blue", call: failing, fallback }).decide(
      view,
      offered,
      "what now?",
    );
    expect(chosen.optionId).toBe("a");
  });
});

// ── Whole games ────────────────────────────────────────────────────────────

describe("whole games with Jev deciding everything", () => {
  /** Answers every question with a reproducible pseudo-random opinion. */
  function opinionated(seed: string): JevCall {
    const rng = createRng(seed);
    return async ({ questions }) => {
      const answers: Record<string, JevAnswer> = {};
      for (const [key, question] of Object.entries(questions)) {
        if (question.type === "noul") {
          answers[key] = { type: "noul", noul: rng.int(100) / 100 };
        } else {
          const keys = Object.keys(question.criteria);
          const choice = keys[rng.int(keys.length)];
          answers[key] = { type: "choice", choice, probabilities: { [choice]: 0.6 }, confidence: 0.6 };
        }
      }
      return { answers, latencyMs: 1 };
    };
  }

  const EVERYTHING = withModules(HOUSE_V1, {
    reactionFire: true,
    counteraction: true,
    contactHalt: true,
  });

  it("plays to the end through the orders sequence", async () => {
    const log = new EventLog();
    let game = startGame(scenarioFactory(MEETING_ENGAGEMENT_V1, EVERYTHING)());
    const cfg = {
      ruleset: EVERYTHING,
      terrain: proceduralTerrain(STANDARD_GROUND),
      commanders: {
        blue: jevOrdersCommander({ side: "blue", call: opinionated("b") }),
        red: jevOrdersCommander({ side: "red", call: opinionated("r") }),
      },
      tactical: {
        blue: jevTacticalDecider({ side: "blue", call: opinionated("bt") }),
        red: jevTacticalDecider({ side: "red", call: opinionated("rt") }),
      },
      tacticalPositions: { blue: true, red: true },
      rng: createRng("jev:dice"),
      log,
      maxTurns: 12,
    };
    while (!game.over) game = await advanceTurn(game, cfg);
    // Jev took activations and the new positions were on offer.
    const all = game.turns.flatMap((turn) => turn.decisions);
    expect(all.some((d) => d.question === "which element acts now, and how?")).toBe(true);
    expect(all.some((d) => d.options.some((o) => o.id.includes(":pos:")))).toBe(true);

    expect(game.turns.length).toBeGreaterThan(0);
    const jevCalls = game.turns.flatMap((turn) => turn.decisions).filter((d) => d.chosenBy === "jev");
    expect(jevCalls.some((d) => d.phase === "arcReaction")).toBe(true);
  }, 60_000);

  it("plays to the end through the activation sequence", async () => {
    const rng = createRng("jev:arc");
    const outcome = await runGame(scenarioFactory(MEETING_ENGAGEMENT_V1, EVERYTHING)(), {
      ruleset: EVERYTHING,
      terrain: proceduralTerrain(STANDARD_GROUND),
      commanders: {
        blue: jevCommander({ side: "blue", call: opinionated("b"), fallback: heuristicCommander("blue", rng) }),
        red: jevCommander({ side: "red", call: opinionated("r"), fallback: heuristicCommander("red", rng) }),
      },
      tactical: {
        blue: jevTacticalDecider({ side: "blue", call: opinionated("bt") }),
        red: jevTacticalDecider({ side: "red", call: opinionated("rt") }),
      },
      rng,
      log: new EventLog(),
      maxTurns: 12,
    });
    expect(outcome.turns).toBeGreaterThan(0);
  }, 60_000);
});

// ── Spotting, and the choices nobody used to be asked ──────────────────────

describe("Jev choosing who tries to spot a concealed element", () => {
  const CONCEALMENT = withModules(HOUSE_V1, { concealment: true });
  // B1 is concealed and acting; R-near is 600 m off, R-far 1.5 km.
  const scene = () =>
    board(
      [
        fe("B1", "blue", at(0, 0), { concealed: true }),
        fe("Rnear", "red", at(0, 600)),
        fe("Rfar", "red", at(0, 1500)),
      ],
      false,
    );
  const sightings = (log: EventLog) =>
    log.all().filter((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "sighting");

  it("lets the nearest look when no decider is configured, as the rule reads it", async () => {
    const cfg = config({}, CONCEALMENT);
    await attemptSightingInterruptLive(scene(), "B1", "blue", cfg, 2);
    expect(sightings(cfg.log)[0].actorIds).toEqual(["Rnear"]);
  });

  it("lets Jev pick a different observer", async () => {
    // Observers are keyed nearest first: w0 = Rnear, w1 = Rfar.
    const call = fakeJev(() => ({
      decision: { type: "choice", choice: "w1", probabilities: { w0: 0.2, w1: 0.8 }, confidence: 0.6 },
    }));
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call }) } }, CONCEALMENT);
    await attemptSightingInterruptLive(scene(), "B1", "blue", cfg, 2);
    expect(sightings(cfg.log)[0].actorIds).toEqual(["Rfar"]);
    expect(decisions(cfg.log)[0]).toMatchObject({ chosenBy: "jev", chosenId: "Rfar" });
    // The concealed element's identity is what the attempt is FOR.
    expect(JSON.stringify(call.requests[0].state)).not.toContain('"B1"');
  });

  it("falls back to the nearest when Jev cannot be reached", async () => {
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: failing }) } }, CONCEALMENT);
    await attemptSightingInterruptLive(scene(), "B1", "blue", cfg, 2);
    expect(sightings(cfg.log)[0].actorIds).toEqual(["Rnear"]);
    expect(decisions(cfg.log)[0].fallback).toBe("error");
  });
});

describe("a choice the engine used to make by heuristic", () => {
  const state = board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1200))]);
  const options = [
    { id: "fire", kind: "fire" as const, actorId: "B1", targetId: "R1", summary: "B1 fires at R1" },
  ];
  const ask = (answer: JevAnswer | null, call?: JevCall) =>
    chooseOptionLive(
      state,
      "blue",
      options,
      "reserve at the end of its move?",
      config({
        tactical: {
          blue: jevTacticalDecider({
            side: "blue",
            call: call ?? fakeJev((): Record<string, JevAnswer> => (answer ? { decision: answer } : {})),
          }),
        },
      }),
      2,
      true,
    );

  it("takes Jev's choice", async () => {
    expect((await ask({ type: "choice", choice: "o0", probabilities: {}, confidence: 0.9 }))?.id).toBe("fire");
  });

  it("treats Jev choosing to pass as a pass", async () => {
    expect(await ask({ type: "choice", choice: "pass", probabilities: {}, confidence: 0.9 })).toBeNull();
  });

  it("does NOT treat an unsure or unreachable Jev as a pass — the engine's default decides", async () => {
    // The difference matters: passing is final in the Counteraction Round,
    // and an outage must not be read as a decision to stand down.
    expect(await ask({ type: "choice", choice: "pass", probabilities: {}, confidence: 0.01 })).toBeUndefined();
    expect(await ask(null, failing)).toBeUndefined();
  });

  it("asks nobody when no decider is configured", async () => {
    expect(await chooseOptionLive(state, "blue", options, "?", config(), 2, true)).toBeUndefined();
  });
});

describe("Jev's calls in the turn's playback", () => {
  it("records a step for each call, so a held shot is visible", async () => {
    const labels: string[] = [];
    const cfg = config({
      onStep: (step) => labels.push(step.label),
      tactical: { red: jevTacticalDecider({ side: "red", call: answers("none") }) },
    });
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    expect(labels.some((label) => label.includes("none") && label.includes("Jev 80%"))).toBe(true);
  });
});

describe("a trial with Jev on the challenger's side", () => {
  it("counts Jev's calls, and only the challenger's", async () => {
    const result = await runTrial({
      ruleset: withModules(HOUSE_V1, { reactionFire: true, contactHalt: true }),
      terrain: proceduralTerrain(STANDARD_GROUND),
      scenario: scenarioFactory(MEETING_ENGAGEMENT_V1, withModules(HOUSE_V1, { reactionFire: true })),
      seeds: ["t1"],
      maxTurns: 6,
      challenger: (side) => heuristicOrdersCommander(side),
      baseline: (side) => heuristicOrdersCommander(side),
      challengerTactics: (side): TacticalDecider =>
        jevTacticalDecider({
          side,
          // Yes to every yes/no, the first option of every choice, confidently.
          call: fakeJev((questions) =>
            Object.fromEntries(
              Object.entries(questions).map(([key, q]): [string, JevAnswer] => [
                key,
                q.type === "noul"
                  ? { type: "noul", noul: 0.9 }
                  : { type: "choice", choice: Object.keys(q.criteria)[0], probabilities: {}, confidence: 0.9 },
              ]),
            ),
          ),
        }),
    });
    expect(result.tacticalCalls).toBeGreaterThan(0);
    expect(result.tacticalFallbacks).toBe(0);
  }, 60_000);
});

// ── The console ────────────────────────────────────────────────────────────

describe("every decision is printed to the console", () => {
  it("prints a headline saying who decided what, and how sure", async () => {
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: answers("none") }) } });
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    const lines = vi.mocked(console.groupCollapsed).mock.calls.map((call) => String(call[0]));
    expect(lines.some((line) => line.includes("[Jev red]") && line.includes("who fires at B1?") && line.includes("Jev 80%"))).toBe(true);
  });

  it("prints nothing when told not to", async () => {
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: answers("none"), log: false }) } });
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    expect(console.groupCollapsed).not.toHaveBeenCalled();
  });
});

// ── Escalation and sampling (item 5) ───────────────────────────────────────

describe("when Jev is unsure", () => {
  it("asks the side's language model, and says so", async () => {
    const escalate = vi.fn(async () => '{"choice":"f0","why":"the flank shot is worth it"}');
    const cfg = config({
      tactical: { red: jevTacticalDecider({ side: "red", call: answers("none", 0.05), escalate }) },
    });
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    expect(escalate).toHaveBeenCalledOnce();
    expect(reactions(cfg.log)).toHaveLength(1);
    expect(decisions(cfg.log)[0]).toMatchObject({ chosenBy: "llm", chosenId: "R1" });
    expect(decisions(cfg.log)[0].rationale).toMatch(/flank shot/);
  });

  it("falls back to the rule if the model's answer is not one of the options", async () => {
    const cfg = config({
      tactical: {
        red: jevTacticalDecider({ side: "red", call: answers("none", 0.05), escalate: async () => "attack!" }),
      },
    });
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", cfg, 2, standing("red", "R1", "always"), "actionReaction");
    expect(decisions(cfg.log)[0]).toMatchObject({ chosenBy: "heuristic", fallback: "lowConfidence" });
    expect(reactions(cfg.log)).toHaveLength(1);
  });
});

describe("sampling from Jev's probabilities", () => {
  const even = fakeJev((): Record<string, JevAnswer> => ({
    decision: { type: "choice", choice: "none", probabilities: { none: 0.5, f0: 0.5 }, confidence: 0.5 },
  }));
  const run = async (seed: string) => {
    const cfg = config({
      tactical: { red: jevTacticalDecider({ side: "red", call: even, sampleRng: createRng(seed) }) },
    });
    await reactiveFireLive(board([mover(), watcher()]), "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    return decisions(cfg.log)[0].chosenId;
  };

  it("takes a 50/50 call both ways across seeds", async () => {
    const outcomes = new Set<string>();
    for (let i = 0; i < 20; i += 1) outcomes.add(await run(`s${i}`));
    expect([...outcomes].sort()).toEqual(["R1", "none"]);
  });

  it("takes it the same way every time for the same seed", async () => {
    expect(await run("fixed")).toBe(await run("fixed"));
  });
});

// ── Richer state (item 3) ──────────────────────────────────────────────────

describe("what Jev is told about the wider situation", () => {
  it("includes the threat to each reactor, the objective and who can support it", () => {
    const state = {
      ...board([mover(), watcher(), fe("R2", "red", at(100, 1000))]),
      objectives: { blue: at(0, 3000), red: at(0, 1100) },
    };
    const picture = reactionState({
      state,
      config: config(),
      turn: 2,
      round: "actionReaction",
      side: "red",
      actorId: "B1",
      wasFiredUpon: false,
      candidates: [{ reactorId: "R1", rangeM: 1000, capability: "atk", engage: "always", ruleSaysReact: true }],
      maxReactors: 1,
    });
    const r1 = picture.reactors[0] as Record<string, unknown>;
    expect(r1.threatsToYou).toEqual([expect.objectContaining({ from: "B1" })]);
    expect(r1.objective).toMatchObject({ distanceM: expect.any(Number) });
    expect(r1.canActTogetherWith).toEqual(["R2"]);
  });

  it("recalls recent exchanges without naming an enemy nobody has seen", () => {
    const cfg = config();
    cfg.log.append({
      type: "resolution",
      turn: 1,
      phase: "arcAction",
      kind: "directFire",
      rulesetId: "x",
      actorIds: ["R1"],
      targetIds: ["B1"],
      modifiers: [],
      result: "oneHit",
      effects: [],
    });
    const unseenByBlue = board([mover(), watcher()], false);
    expect(recentEvents(unseenByBlue, cfg, "blue")).toEqual([
      expect.objectContaining({ by: ["unseen enemy"], at: ["B1"], result: "oneHit" }),
    ]);
  });
});

// ── Terrain-aware positions (item 2) ───────────────────────────────────────

describe("terrain-aware move options", () => {
  const terrain = proceduralTerrain(STANDARD_GROUND);
  const scene = () => board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1800))]);

  it("are not offered unless asked for, so the measured game is unchanged", () => {
    const options = optionsFor(scene(), scene().forceElements.B1, config({ terrain }));
    expect(options.some((option) => option.id.includes(":pos:"))).toBe(false);
  });

  it("are offered per side when asked for, each a real move somewhere else", () => {
    const cfg = config({ terrain, tacticalPositions: { blue: true } });
    const positions = optionsFor(scene(), scene().forceElements.B1, cfg).filter((option) =>
      option.id.includes(":pos:"),
    );
    expect(positions.length).toBeGreaterThan(0);
    for (const option of positions) {
      expect(option.kind).toBe("move");
      expect(distanceM(option.destination!, at(0, 0))).toBeGreaterThan(40);
    }
    // Red was not given them.
    const red = optionsFor(scene(), scene().forceElements.R1, cfg);
    expect(red.some((option) => option.id.includes(":pos:"))).toBe(false);
  });
});

// ── Jev taking the activations (item 1) ────────────────────────────────────

describe("the commander plans, Jev picks each activation", () => {
  const RULES = withModules(HOUSE_V1, {});
  const setup = () =>
    board([
      fe("B1", "blue", at(0, 0)),
      fe("B2", "blue", at(150, 0)),
      fe("R1", "red", at(0, 2500)),
    ]);
  const base = () => ({
    ruleset: RULES,
    terrain: flatTerrain(),
    commanders: { blue: heuristicOrdersCommander("blue"), red: heuristicOrdersCommander("red") },
    rng: createRng("act"),
    log: new EventLog(),
    maxTurns: 10,
  });

  it("may adapt an order to what has happened — here, B2 holds instead", async () => {
    const cfg = base();
    const planned = await planOrdersTurn(setup(), cfg);
    expect(planned.accepted.blue.map((intent) => intent.actorId)).toContain("B2");

    const call = answers((criteria) =>
      Object.keys(criteria).find((key) => criteria[key].startsWith("B2 acts now: B2 holds"))!,
    );
    await executePlannedTurn(planned, { ...cfg, tactical: { blue: jevTacticalDecider({ side: "blue", call }) } });

    const picked = decisions(cfg.log).find((d) => d.question === "which element acts now, and how?");
    expect(picked).toMatchObject({ chosenBy: "jev", chosenId: "B2::B2:hold", side: "blue" });
  });

  it("carries out the orders as written when Jev cannot answer", async () => {
    const cfg = base();
    const planned = await planOrdersTurn(setup(), cfg);
    await executePlannedTurn(planned, {
      ...cfg,
      tactical: { blue: jevTacticalDecider({ side: "blue", call: failing }) },
    });
    const ordered = decisions(cfg.log)
      .filter((d) => d.question === "orders for the turn" && d.side === "blue")
      .map((d) => d.chosenId);
    expect(ordered).toEqual(planned.accepted.blue.map((intent) => intent.optionId));
  });
});

// ── Speed and memory (item 7) ──────────────────────────────────────────────

describe("reactions asked ahead of time", () => {
  it("answers the moment from the prefetch, without asking again", async () => {
    const call = answers("none");
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call }) } });
    const state = board([mover(), watcher()]);

    await prefetchReactionsFor(state, cfg, noStandingOrders(), [{ actorId: "B1", side: "blue" }], 2);
    expect(call.requests).toHaveLength(1);
    expect(Object.keys(call.requests[0].questions)).toEqual(["m0"]);

    await reactiveFireLive({ ...state, phase: "arcReaction" }, "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    expect(call.requests).toHaveLength(1);
    expect(decisions(cfg.log)[0].rationale).toMatch(/ahead of time/);
  });

  it("asks live when the moment turned out differently", async () => {
    const call = answers("none");
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call }) } });
    await prefetchReactionsFor(board([mover(), watcher()]), cfg, noStandingOrders(), [{ actorId: "B1", side: "blue" }], 2);
    // B1 is somewhere else by the time it acts.
    const moved = board([fe("B1", "blue", at(0, 300)), watcher()]);
    await reactiveFireLive(moved, "B1", "blue", cfg, 2, noStandingOrders(), "actionReaction");
    expect(call.requests).toHaveLength(2);
  });
});

describe("the persistent answer cache", () => {
  const memory = () => {
    const store = new Map<string, string>();
    return { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
  };
  const request: JevRequest = { state: { a: 1 }, questions: { q: { type: "noul", instructions: "?" } } };

  it("asks once, across page loads, and does not charge twice", async () => {
    const storage = memory();
    const call = vi.fn(async () => ({ answers: { q: { type: "noul", noul: 0.7 } as JevAnswer }, latencyMs: 90, costUsd: 0.0001 }));
    await withPersistentCache(call, { namespace: "m", storage })(request);
    const again = await withPersistentCache(call, { namespace: "m", storage })(request);
    expect(call).toHaveBeenCalledOnce();
    expect(again).toEqual({ answers: { q: { type: "noul", noul: 0.7 } }, latencyMs: 0 });
  });

  it("keeps models apart", async () => {
    const storage = memory();
    const call = vi.fn(async () => ({ answers: {}, latencyMs: 1 }));
    await withPersistentCache(call, { namespace: "jev-1.13", storage })(request);
    await withPersistentCache(call, { namespace: "jev-2", storage })(request);
    expect(call).toHaveBeenCalledTimes(2);
  });
});

describe("decision marks on the map", () => {
  const step = (side: Side, actorId: string, extra: Partial<TurnStep> = {}): TurnStep => ({
    turn: 2,
    phase: "arcReaction",
    label: `${actorId}: who fires? \u2192 none (Jev 80%)`,
    state: board([mover(), watcher()]),
    side,
    actorId,
    decision: { chosenId: "none", chosenBy: "jev", p: 0.8, mark: "holds fire on B1" },
    ...extra,
  });

  it("shows a side its own decisions", () => {
    const marks = decisionMarksFor([step("red", "R1")], -1, "red");
    expect(marks.get("R1")).toMatchObject({ text: "holds fire on B1 80%", fallback: false });
  });

  it("never shows a side what the enemy decided", () => {
    expect(decisionMarksFor([step("red", "R1")], -1, "blue").size).toBe(0);
  });

  it("only counts decisions up to the step on screen", () => {
    const steps = [step("red", "R1", { label: "Sighting", decision: undefined, actorId: undefined, side: undefined }), step("red", "R1")];
    expect(decisionMarksFor(steps, 0, "both").size).toBe(0);
    expect(decisionMarksFor(steps, 1, "both").size).toBe(1);
  });
});

// ── Jev's read, handed to the planner (item 5) ─────────────────────────────

describe("Jev's assessment for the planner", () => {
  it("puts a danger and opportunity score for each element in front of the planner", async () => {
    const state = board([mover(), watcher()]);
    const call = fakeJev((questions) =>
      Object.fromEntries(
        Object.keys(questions).map((key): [string, JevAnswer] => [
          key,
          { type: "score", score: key.endsWith("danger") ? 4.2 : 1.5, probabilities: {}, confidence: 0.6 },
        ]),
      ),
    );
    let seen: OrdersRequest | undefined;
    const inner = {
      kind: "llm" as const,
      name: "planner",
      planTurn: async (request: OrdersRequest) => {
        seen = request;
        return { side: "blue" as const, intents: [] };
      },
    };
    const request: OrdersRequest = {
      side: "blue",
      turn: 2,
      view: projectForSide(state, "blue"),
      optionsByElement: {},
      activationBudget: 2,
      reserveLimit: 0,
    };
    await withJevAssessments(inner, { side: "blue", call }).planTurn(request);

    expect(seen?.assessments).toEqual({ B1: { danger: 4.2, opportunity: 1.5 } });
    expect(buildOrdersPrompt(seen!)).toContain("B1  danger 4.2  opportunity 1.5");
  });

  it("plans without it when Jev cannot be reached", async () => {
    let seen: OrdersRequest | undefined;
    const inner = {
      kind: "llm" as const,
      name: "planner",
      planTurn: async (request: OrdersRequest) => {
        seen = request;
        return { side: "blue" as const, intents: [] };
      },
    };
    await withJevAssessments(inner, { side: "blue", call: failing }).planTurn({
      side: "blue",
      turn: 2,
      view: projectForSide(board([mover(), watcher()]), "blue"),
      optionsByElement: {},
      activationBudget: 2,
      reserveLimit: 0,
    });
    expect(seen?.assessments).toBeUndefined();
  });
});

// ── Checkpoints inside a move ──────────────────────────────────────────────

describe("a move stops to ask when something happens to it", () => {
  const eastward = (to = 3000) => ({
    id: "B1:move",
    kind: "move" as const,
    actorId: "B1",
    destination: at(to, 0),
    summary: "B1 advances east",
  });
  /** A Jev that answers every move question with `answer`, and records the questions. */
  const moveJev = (answer: "press" | "halt" | "cover") => answers(answer);
  const asked = (log: EventLog) => decisions(log).map((d) => d.question);

  describe("walking into an identified enemy's sight and range", () => {
    // R1 is identified, 5.5 km east: out of reach at the start (3 km guns),
    // in reach about 2.5 km into the move.
    const scene = () => board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(5500, 0))]);

    it("stops where it comes into reach when Jev says halt", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("halt") }) } }, HOUSE_V1);
      const next = await resolveMoveLive(scene(), eastward(), cfg, 2);
      const stoppedAt = distanceM(next.forceElements.B1.position, at(0, 0));
      expect(stoppedAt).toBeGreaterThan(2000);
      expect(stoppedAt).toBeLessThan(2900);
      expect(decisions(cfg.log)[0]).toMatchObject({
        question: "moving into enemy sight: carry on, halt, or break for cover?",
        chosenBy: "jev",
        chosenId: "halt",
      });
    });

    it("asks once per enemy, and goes all the way when Jev says carry on", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("press") }) } }, HOUSE_V1);
      const next = await resolveMoveLive(scene(), eastward(), cfg, 2);
      expect(distanceM(next.forceElements.B1.position, at(3000, 0))).toBeLessThan(5);
      expect(asked(cfg.log)).toHaveLength(1);
    });

    it("carries on, as it always did, when Jev cannot answer", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: failing }) } }, HOUSE_V1);
      const next = await resolveMoveLive(scene(), eastward(), cfg, 2);
      expect(distanceM(next.forceElements.B1.position, at(3000, 0))).toBeLessThan(5);
      expect(decisions(cfg.log)[0]).toMatchObject({ chosenId: "press", fallback: "error" });
    });

    it("does not ask about an enemy that could already reach it where it started", async () => {
      const near = board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(2000, 0))]);
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("halt") }) } }, HOUSE_V1);
      const next = await resolveMoveLive(near, eastward(1000), cfg, 2);
      expect(asked(cfg.log)).toHaveLength(0);
      expect(distanceM(next.forceElements.B1.position, at(1000, 0))).toBeLessThan(5);
    });
  });

  describe("shot at as it set off", () => {
    const scene = () => board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(0, 1500))]);

    it("changes nothing without a decider", async () => {
      const plain = config({}, HOUSE_V1);
      const live = config({}, HOUSE_V1);
      const a = resolveAction(scene(), eastward(), plain, 2);
      const b = await resolveMoveLive(scene(), eastward(), live, 2, { tookFire: ["R1"] });
      expect(b).toEqual(a);
    });

    it("stays put when Jev says halt", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("halt") }) } }, HOUSE_V1);
      const next = await resolveMoveLive(scene(), eastward(), cfg, 2, { tookFire: ["R1"] });
      expect(next.forceElements.B1.position).toEqual(at(0, 0));
      expect(decisions(cfg.log)[0]).toMatchObject({
        question: "under fire: carry on, halt, or break for cover?",
        chosenId: "halt",
      });
    });

    it("breaks off into the nearest cover when Jev says so", async () => {
      // A wood 300 m north of the start; open ground everywhere else.
      const woods = {
        groundHeightM: () => 0,
        classify: (point: LatLng) => (point.lat > at(0, 250).lat && point.lat < at(0, 900).lat ? "woodsLight" : "open"),
      } as const;
      const cfg = config(
        { terrain: woods, tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("cover") }) } },
        HOUSE_V1,
      );
      const next = await resolveMoveLive(scene(), eastward(), cfg, 2, { tookFire: ["R1"] });
      const end = next.forceElements.B1.position;
      expect(woods.classify(end)).toBe("woodsLight");
      expect(distanceM(end, at(0, 0))).toBeLessThan(750);
      expect(decisions(cfg.log)[0].options.map((o) => o.id)).toContain("cover");
    });

    it("does not offer cover when there is none", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("halt") }) } }, HOUSE_V1);
      await resolveMoveLive(scene(), eastward(), cfg, 2, { tookFire: ["R1"] });
      expect(decisions(cfg.log)[0].options.map((o) => o.id)).toEqual(["press", "halt"]);
    });
  });

  describe("a friend lost close by this turn", () => {
    const scene = () =>
      board([fe("B1", "blue", at(0, 0)), fe("B2", "blue", at(300, 0), { combatStrength: 0 }), fe("R1", "red", at(0, 9000))]);
    const lost = (cfg: PhaseConfig) =>
      cfg.log.append({
        type: "resolution",
        turn: 2,
        phase: "arcAction",
        kind: "directFire",
        rulesetId: "x",
        actorIds: ["R1"],
        targetIds: ["B2"],
        modifiers: [],
        result: "threeHits",
        effects: [{ kind: "eliminated", feId: "B2" }],
      });

    it("asks before setting off, and only once a turn", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("press") }) } }, HOUSE_V1);
      lost(cfg);
      const once = await resolveMoveLive(scene(), eastward(1000), cfg, 2);
      await resolveMoveLive(once, { ...eastward(1500), id: "B1:again" }, cfg, 2);
      expect(asked(cfg.log).filter((q) => q.startsWith("setback nearby"))).toHaveLength(1);
      expect(decisions(cfg.log)[0].rationale ?? "").not.toMatch(/Jev unavailable/);
    });

    it("ignores losses from earlier turns", async () => {
      const cfg = config({ tactical: { blue: jevTacticalDecider({ side: "blue", call: moveJev("halt") }) } }, HOUSE_V1);
      lost(cfg);
      await resolveMoveLive(scene(), eastward(1000), cfg, 3);
      expect(asked(cfg.log)).toHaveLength(0);
    });
  });
});

// ── The planner's prompt when Jev carries the orders out ───────────────────

describe("the orders prompt with and without Jev", () => {
  const request: OrdersRequest = {
    side: "blue",
    turn: 2,
    view: projectForSide(board([mover(), watcher()]), "blue"),
    optionsByElement: {},
    activationBudget: 2,
    reserveLimit: 0,
  };

  it("is unchanged with Jev off: standing orders are final, orders run as written", () => {
    const prompt = buildOrdersPrompt(request);
    expect(prompt).toContain("You are not asked again when the moment comes");
    expect(prompt).not.toContain("HOW YOUR ORDERS ARE CARRIED OUT");
    expect(prompt).toBe(buildOrdersPrompt(request, undefined, { jev: false }));
  });

  it("with Jev on, says orders are intent and standing orders are defaults — except never", () => {
    const prompt = buildOrdersPrompt(request, undefined, { jev: true });
    expect(prompt).toContain("HOW YOUR ORDERS ARE CARRIED OUT");
    expect(prompt).toContain('except "never", which it must obey');
    expect(prompt).toContain('"why":"what this element is for"');
    expect(prompt).not.toContain("You are not asked again");
  });
});
