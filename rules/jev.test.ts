/**
 * Jev as a decider: the question, the answer, and what happens when there is
 * no answer.
 *
 * No network. The model call is a fake that answers from a function, which
 * is the point of injecting it — every rule about what Jev may and may not
 * decide can be asserted without one.
 */

import { describe, expect, it } from "vitest";

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
import {
  noStandingOrders,
  reactiveFireLive,
  resolveAction,
  resolveMoveLive,
  runGame,
  runReactiveFire,
  type PhaseConfig,
} from "./turnLoop";

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
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: fakeJev(everyNoul(0.1)) }) } });
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
    expect(decision).toMatchObject({ chosenBy: "jev", chosenId: "hold", actorId: "R1" });
    expect(decision.probabilities?.fire).toBeCloseTo(0.1);
  });

  it("takes a shot the declared rules would have let pass, when Jev says yes", async () => {
    // ifFiredUpon, and the mover is only moving: the rule holds. Jev may not.
    const cfg = config({ tactical: { red: jevTacticalDecider({ side: "red", call: fakeJev(everyNoul(0.9)) }) } });
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

  it("asks about every eligible reactor in ONE request", async () => {
    const call = fakeJev(everyNoul(0.8));
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
    expect(Object.keys(call.requests[0].questions)).toHaveLength(3);
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
      rng: createRng("jev:dice"),
      log,
      maxTurns: 12,
    };
    while (!game.over) game = await advanceTurn(game, cfg);

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
