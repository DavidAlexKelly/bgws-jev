import { describe, expect, it } from "vitest";

import {
  advanceTurn,
  discardPending,
  displayState,
  executePending,
  planNextTurn,
  startGame,
} from "../lib/liveGame";
import { scenarioFactory } from "../lib/forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import { createRng } from "./dice";
import { EventLog } from "./events";
import { MEETING_ENGAGEMENT_V1 } from "./forceList";
import {
  buildOrdersPrompt,
  llmCommander,
  parseOrdersReply,
  unavailableModelCall,
} from "./llmCommander";
import {
  executePlannedTurn,
  heuristicOrdersCommander,
  ordersRequestFor,
  planOrdersTurn,
  runOrdersTurn,
  validateOrders,
  type Orders,
  type OrdersCommander,
} from "./orders";
import { HOUSE_V1 } from "./ruleset";

const terrain = proceduralTerrain(STANDARD_GROUND);

function config(commanders: Record<"blue" | "red", OrdersCommander>) {
  return {
    ruleset: HOUSE_V1,
    terrain,
    commanders,
    rng: createRng("orders-test"),
    log: new EventLog(),
    maxTurns: 40,
  };
}

const setup = () => scenarioFactory(MEETING_ENGAGEMENT_V1, HOUSE_V1)();

describe("the orders request", () => {
  it("offers options grouped by element, fog-of-war filtered", () => {
    const request = ordersRequestFor(setup(), "blue", config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    }));

    expect(Object.keys(request.optionsByElement).length).toBeGreaterThan(0);
    for (const [actorId, options] of Object.entries(request.optionsByElement)) {
      expect(options.length).toBeGreaterThan(0);
      for (const option of options) expect(option.actorId).toBe(actorId);
    }
    // Own force is complete; enemies only as sighted.
    expect(request.view.own.length).toBeGreaterThan(0);
    expect(request.view.own.every((fe) => fe.side === "blue")).toBe(true);
  });

  it("omits elements whose only option is to hold", () => {
    // A prompt should be about the units with a decision to make.
    const request = ordersRequestFor(setup(), "blue", config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    }));
    for (const options of Object.values(request.optionsByElement)) {
      expect(options.some((option) => option.kind !== "hold")).toBe(true);
    }
  });
});

describe("validateOrders is the guard against an invented move", () => {
  const request = () =>
    ordersRequestFor(setup(), "blue", config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    }));

  it("accepts an option that was offered", () => {
    const req = request();
    const [actorId, options] = Object.entries(req.optionsByElement)[0];
    const orders: Orders = {
      side: "blue",
      intents: [{ actorId, optionId: options[0].id }],
    };
    const { accepted, rejected } = validateOrders(orders, req);
    expect(accepted).toHaveLength(1);
    expect(rejected).toHaveLength(0);
  });

  it("rejects an option that was never offered", () => {
    // THE LOAD-BEARING CASE. A model that invents a move must produce a unit
    // that did nothing, never a rule violation the engine tries to honour.
    const req = request();
    const [actorId] = Object.entries(req.optionsByElement)[0];
    const { accepted, rejected } = validateOrders(
      { side: "blue", intents: [{ actorId, optionId: `${actorId}:teleport:moon` }] },
      req,
    );
    expect(accepted).toHaveLength(0);
    expect(rejected[0].reason).toMatch(/not a legal option/);
  });

  it("rejects an element that does not exist", () => {
    const req = request();
    const { rejected } = validateOrders(
      { side: "blue", intents: [{ actorId: "ghost-99", optionId: "whatever" }] },
      req,
    );
    expect(rejected[0].reason).toMatch(/no such element/);
  });

  it("accepts move THEN engage for one element", () => {
    // The rules allow it at the firerMoved penalty, and this validator used
    // to reject it as a duplicate. Found by GPT-5.2, which ordered a platoon
    // to close and engage: the model was right and the validator was wrong.
    const req = request();
    const entry = Object.entries(req.optionsByElement).find(
      ([, options]) =>
        options.some((o) => o.kind === "move") &&
        options.some((o) => o.kind === "fire" || o.kind === "assault"),
    );
    if (!entry) return; // no element can do both this turn; nothing to assert
    const [actorId, options] = entry;

    const move = options.find((o) => o.kind === "move")!;
    const engage = options.find((o) => o.kind === "fire" || o.kind === "assault")!;

    const { accepted, rejected } = validateOrders(
      { side: "blue", intents: [{ actorId, optionId: move.id }, { actorId, optionId: engage.id }] },
      req,
    );
    expect(accepted).toHaveLength(2);
    expect(rejected).toHaveLength(0);
  });

  it("rejects a second MOVE for the same element", () => {
    const req = request();
    const entry = Object.entries(req.optionsByElement).find(
      ([, options]) => options.filter((o) => o.kind === "move").length >= 2,
    );
    if (!entry) return;
    const [actorId, options] = entry;
    const moves = options.filter((o) => o.kind === "move");

    const { accepted, rejected } = validateOrders(
      {
        side: "blue",
        intents: [{ actorId, optionId: moves[0].id }, { actorId, optionId: moves[1].id }],
      },
      req,
    );
    expect(accepted).toHaveLength(1);
    expect(rejected[0].reason).toMatch(/only move once/);
  });

  it("rejects a third order, and a second after engaging", () => {
    const req = request();
    const [actorId, options] = Object.entries(req.optionsByElement)[0];
    const engage =
      options.find((o) => o.kind === "fire" || o.kind === "assault") ?? options[0];

    const { accepted, rejected } = validateOrders(
      {
        side: "blue",
        intents: [{ actorId, optionId: engage.id }, { actorId, optionId: options[0].id }],
      },
      req,
    );
    expect(accepted).toHaveLength(1);
    expect(rejected[0].reason).toMatch(/already ordered/);
  });

  it("counts command capacity in ELEMENTS, not orders", () => {
    // A move-then-fire pair is one element committed, not two. Counting
    // orders would make command capacity quietly depend on how a commander
    // phrased its plan.
    const req = { ...request(), activationBudget: 1 };
    const entry = Object.entries(req.optionsByElement).find(
      ([, options]) =>
        options.some((o) => o.kind === "move") &&
        options.some((o) => o.kind === "fire" || o.kind === "assault"),
    );
    if (!entry) return;
    const [actorId, options] = entry;
    const move = options.find((o) => o.kind === "move")!;
    const engage = options.find((o) => o.kind === "fire" || o.kind === "assault")!;

    const { accepted } = validateOrders(
      { side: "blue", intents: [{ actorId, optionId: move.id }, { actorId, optionId: engage.id }] },
      req,
    );
    expect(accepted).toHaveLength(2);
  });

  it("rejects orders beyond the command budget", () => {
    const req = {
      ...request(),
      activationBudget: 1,
    };
    const entries = Object.entries(req.optionsByElement).slice(0, 3);
    const { accepted, rejected } = validateOrders(
      {
        side: "blue",
        intents: entries.map(([actorId, options]) => ({ actorId, optionId: options[0].id })),
      },
      req,
    );
    expect(accepted).toHaveLength(1);
    expect(rejected.some((entry) => /command capacity/.test(entry.reason))).toBe(true);
  });
});

describe("a game played by orders", () => {
  it("plays turn by turn and reaches a conclusion", async () => {
    const cfg = config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    });
    let game = startGame(setup());
    let guard = 0;
    while (!game.over && guard < 60) {
      game = await advanceTurn(game, cfg);
      guard += 1;
    }
    expect(game.over).toBe(true);
    expect(game.turns.length).toBeGreaterThan(0);
    expect(["annihilation", "turnLimit"]).toContain(game.reason);
  }, 40_000);

  it("loses strength over the game", async () => {
    const cfg = config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    });
    let game = startGame(setup());
    for (let i = 0; i < 6 && !game.over; i += 1) game = await advanceTurn(game, cfg);

    const first = game.turns[0].strength;
    const last = game.turns[game.turns.length - 1].strength;
    expect(last.blue + last.red).toBeLessThan(first.blue + first.red);
  }, 40_000);

  it("does not advance once the game is over", async () => {
    const cfg = config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    });
    let game = startGame(setup());
    let guard = 0;
    while (!game.over && guard < 60) {
      game = await advanceTurn(game, cfg);
      guard += 1;
    }
    const again = await advanceTurn(game, cfg);
    expect(again.turns.length).toBe(game.turns.length);
  }, 60_000);

  it("asks each commander exactly once per turn", async () => {
    // The whole reason the orders phase exists: two model calls a turn, not
    // fifty. If this regresses, an LLM game becomes unaffordable.
    let blueCalls = 0;
    let redCalls = 0;
    const counting = (side: "blue" | "red"): OrdersCommander => ({
      kind: "heuristic",
      name: `count-${side}`,
      async planTurn(request) {
        if (side === "blue") blueCalls += 1;
        else redCalls += 1;
        return heuristicOrdersCommander(side).planTurn(request);
      },
    });

    const cfg = config({ blue: counting("blue"), red: counting("red") });
    let game = startGame(setup());
    for (let i = 0; i < 3; i += 1) game = await advanceTurn(game, cfg);

    expect(blueCalls).toBe(3);
    expect(redCalls).toBe(3);
  }, 40_000);
});

describe("planning and executing are the same turn", () => {
  const both = () => ({
    blue: heuristicOrdersCommander("blue"),
    red: heuristicOrdersCommander("red"),
  });
  const totalCs = (state: ReturnType<typeof setup>) =>
    Object.values(state.forceElements).reduce((sum, fe) => sum + fe.combatStrength, 0);

  it("composes into the same turn, whichever entry point is used", async () => {
    // ⚠ THIS CANNOT FAIL, AND IT IS STILL WORTH HAVING.
    //
    // `runOrdersTurn` IS `executePlannedTurn(planOrdersTurn(x))` — so this
    // compares the composition against itself and is a tautology today. It is
    // here as a tripwire: if anyone ever reimplements runOrdersTurn as a
    // second copy of the sequence rather than a composition of the two halves,
    // this starts doing real work and will catch the drift.
    //
    // The test that actually protects reports/ is the characterisation one
    // below, whose numbers were captured from master BEFORE the split.
    //
    // Two separate configs: the rng and the log are stateful, so running both
    // paths against one config would feed the second path different dice.
    const state = setup();
    const whole = await runOrdersTurn(state, config(both()));
    const halves = config(both());
    const split = await executePlannedTurn(await planOrdersTurn(state, halves), halves);

    expect(split.state).toEqual(whole.state);
    expect(split.orders).toEqual(whole.orders);
    expect(split.rejected).toEqual(whole.rejected);
    expect(split.counteraction).toEqual(whole.counteraction);
  }, 40_000);

  it("plays a fixed-seed game exactly as it did before the split", async () => {
    // ⚠ THE TEST THAT ACTUALLY PROTECTS reports/.
    //
    // Every measured number in reports/ was produced through this sequence, so
    // the split had to be provably behaviour-neutral rather than merely
    // believed to be. These figures were captured by running the identical
    // scenario against master's engine — before the split, with the plan and
    // execute halves still fused — and they matched the post-split run
    // exactly: same winner, same event count, same combat strength every turn,
    // and the same NUMBER OF STEPS per turn, which is what proves the
    // planning-phase steps (rally, initiative, sighting) are still recorded
    // and still land at the front of the turn.
    //
    // If this fails, the seam moved and the measured results no longer
    // describe the code. Do not update the numbers to make it pass.
    //
    // ⚠ RE-BASELINED ONCE, ON 2026-09-21, AND HERE IS THE EVIDENCE THAT WAS
    // REQUIRED FIRST. The platform snapshot was repointed at the curated L7
    // profile: the Warrior became the FV510 it should always have been
    // (csIndex 8.5 -> 4.3, and its borrowed TOW removed), the two export
    // proxies became the T-72B3 and T-80BVM, and a second Warrior platoon was
    // added to keep the list balanced. Those are INPUTS. The digest below has
    // to move when they move, and refusing to update it would only mean
    // deleting the test.
    //
    // The instruction above still stands, so the seam was shown to be
    // untouched INDEPENDENTLY of the data before these numbers were touched:
    // SYMMETRIC_CONTROL_V1 fields nothing but Challenger 2s, whose only
    // change in that pass was `armourMm`, and whose gun states no penetration
    // figure and therefore fails open regardless of it. Its digest was run
    // against both the old and the new snapshot in one process and came back
    // identical — same winner, same 91 events, same five turn strings. The
    // engine did not move; the force did.
    //
    // The next person to hit this should do the same thing rather than
    // assume: find a scenario your change cannot reach, show ITS digest is
    // unchanged, and only then re-baseline this one.
    const cfg = {
      ruleset: HOUSE_V1,
      terrain,
      commanders: {
        blue: heuristicOrdersCommander("blue", { coLocatedM: HOUSE_V1.coLocatedM }),
        red: heuristicOrdersCommander("red", { coLocatedM: HOUSE_V1.coLocatedM }),
      },
      rng: createRng("digest-a"),
      log: new EventLog(),
      maxTurns: 12,
    };
    let game = startGame(setup());
    while (!game.over) game = await advanceTurn(game, cfg);

    expect(game.turns).toHaveLength(12);
    expect(game.winner).toBe("red");
    expect(game.reason).toBe("annihilation");
    expect(cfg.log.all()).toHaveLength(390);
    expect(
      game.turns.map((t) => `${t.strength.blue}/${t.strength.red}:${t.steps.length}`),
    ).toEqual([
      "52/56:18",
      "37/47:18",
      "23/38:12",
      "20/38:13",
      "20/38:13",
      "8/37:14",
      "8/34:15",
      "7/34:13",
      "5/30:9",
      "5/30:9",
      "5/30:8",
      "0/30:8",
    ]);
  }, 60_000);

  it("planning resolves no combat", async () => {
    // Rally, initiative and sighting happen while planning — they decide what
    // is offered — but nothing may be shot at. A plan that cost combat
    // strength would mean pressing "generate orders" was already playing the
    // turn, and the second button would be a lie.
    const state = setup();
    const planned = await planOrdersTurn(state, config(both()));

    expect(planned.turn).toBe(state.turn);
    expect(planned.state.turn).toBe(state.turn);
    expect(totalCs(planned.state)).toBe(totalCs(state));
  }, 40_000);

  it("offers orders for both sides before either is executed", async () => {
    const planned = await planOrdersTurn(setup(), config(both()));
    expect(planned.accepted.blue.length).toBeGreaterThan(0);
    expect(planned.accepted.red.length).toBeGreaterThan(0);
    expect(planned.initiativeWinner === "blue" || planned.initiativeWinner === "red").toBe(
      true,
    );
  }, 40_000);
});

describe("a turn generated, then executed", () => {
  const both = () => ({
    blue: heuristicOrdersCommander("blue"),
    red: heuristicOrdersCommander("red"),
  });

  it("holds the plan without advancing the game", async () => {
    const cfg = config(both());
    const planned = await planNextTurn(startGame(setup()), cfg);

    expect(planned.pending).not.toBeNull();
    expect(planned.turns).toHaveLength(0);
    // The board on screen is the board the orders were planned against —
    // post-sighting, not the start of the turn. See displayState.
    expect(displayState(planned)).toBe(planned.pending!.planned.state);
  }, 40_000);

  it("does not buy a second opinion when asked twice", async () => {
    // Pressing "generate orders" twice must not throw away the plan that is
    // currently drawn on the map — and must not pay for another model call.
    let calls = 0;
    const counting = (side: "blue" | "red"): OrdersCommander => ({
      kind: "heuristic",
      name: `count-${side}`,
      async planTurn(request) {
        calls += 1;
        return heuristicOrdersCommander(side).planTurn(request);
      },
    });
    const cfg = config({ blue: counting("blue"), red: counting("red") });

    const once = await planNextTurn(startGame(setup()), cfg);
    const twice = await planNextTurn(once, cfg);

    expect(calls).toBe(2); // one per side, not four
    expect(twice.pending).toBe(once.pending);
  }, 40_000);

  it("executing consumes the plan and records the turn", async () => {
    const cfg = config(both());
    const planned = await planNextTurn(startGame(setup()), cfg);
    const played = await executePending(planned, cfg);

    expect(played.pending).toBeNull();
    expect(played.turns).toHaveLength(1);
    expect(played.current).toBe(played.turns[0].state);
    expect(displayState(played)).toBe(played.current);
  }, 40_000);

  it("keeps the planning steps in the turn's playback", async () => {
    // Rally, initiative and sighting are recorded while planning. If they were
    // dropped, a played-back turn would appear to begin at the first shot.
    const cfg = config(both());
    const planned = await planNextTurn(startGame(setup()), cfg);
    const planSteps = planned.pending!.steps.length;
    const played = await executePending(planned, cfg);

    expect(planSteps).toBeGreaterThan(0);
    expect(played.turns[0].steps.length).toBeGreaterThan(planSteps);
    expect(played.turns[0].steps[0].label).toBe(planned.pending!.steps[0].label);
  }, 40_000);

  it("discarding a plan un-writes what planning wrote", async () => {
    // ⚠ PLANNING IS NOT FREE OF SIDE EFFECTS. Rally, initiative and sighting
    // all resolve and all append before either commander is asked. Setting
    // `pending` to null left those behind, so a discarded turn logged "Turn N
    // begins" and an initiative roll that never governed anything.
    const cfg = config(both());
    const start = startGame(setup());
    const before = cfg.log.all().length;

    const planned = await planNextTurn(start, cfg);
    expect(cfg.log.all().length).toBeGreaterThan(before);

    const discarded = discardPending(planned, cfg);
    expect(discarded.pending).toBeNull();
    expect(cfg.log.all()).toHaveLength(before);
    // The board was never wrong -- only the log was.
    expect(discarded.current).toBe(start.current);
    expect(discarded.turns).toHaveLength(0);
  }, 40_000);

  it("re-planning after a discard rolls initiative for the turn once", async () => {
    // ⚠ THIS TEST WAS VACUOUS ON ITS FIRST WRITING, and the rewrite is the
    // point. It looked for a log entry whose label said "begins" -- but
    // "Turn N begins" is a STEP label, and steps are not events. Nothing in
    // the log has a `label` at all, so the filter always found zero and the
    // assertion passed with the bug still present.
    //
    // Initiative is the honest marker: resolveInitiative appends exactly one
    // event per planning phase, tagged `table: "initiative"`. Two of them for
    // one turn is precisely the corruption discarding used to leave behind.
    const cfg = config(both());
    const start = startGame(setup());

    const first = await planNextTurn(start, cfg);
    const second = await planNextTurn(discardPending(first, cfg), cfg);
    const played = await executePending(second, cfg);

    const initiativeRolls = cfg.log
      .all()
      .filter((event) => "table" in event && event.table === "initiative");
    expect(initiativeRolls).toHaveLength(1);
    expect(played.turns).toHaveLength(1);
  }, 60_000);

  it("keeps event sequence numbers contiguous across a discard", async () => {
    // append() numbers by position, so a rewind has to leave no gap and no
    // duplicate -- otherwise a replay keyed on seq silently misreads.
    const cfg = config(both());
    const played = await executePending(
      await planNextTurn(
        discardPending(await planNextTurn(startGame(setup()), cfg), cfg),
        cfg,
      ),
      cfg,
    );
    expect(played.turns).toHaveLength(1);
    expect(cfg.log.all().map((event) => event.seq)).toEqual(
      cfg.log.all().map((_, index) => index),
    );
  }, 60_000);

  it("does not rewind the dice, so a re-ask is a real re-ask", async () => {
    // Deliberate: against a heuristic commander on an unchanged board,
    // rewinding the rng too would hand back the identical plan and the
    // discard button would appear to do nothing.
    const cfg = config(both());
    const start = startGame(setup());
    const first = await planNextTurn(start, cfg);
    const firstWinner = first.pending!.planned.initiativeWinner;
    const firstRejects = first.pending!.planned.rejected.blue.length;

    const second = await planNextTurn(discardPending(first, cfg), cfg);

    // Not asserting the plans DIFFER -- initiative may legitimately land the
    // same way twice. Asserting the second plan is real and complete.
    expect(second.pending).not.toBeNull();
    expect(["blue", "red"]).toContain(second.pending!.planned.initiativeWinner);
    expect(typeof firstWinner).toBe("string");
    expect(firstRejects).toBeGreaterThanOrEqual(0);
  }, 60_000);

  it("discarding nothing is a no-op rather than an error", async () => {
    const cfg = config(both());
    const game = startGame(setup());
    expect(discardPending(game, cfg)).toBe(game);
  });

  it("executing nothing is a no-op rather than an error", async () => {
    const cfg = config(both());
    const game = startGame(setup());
    const played = await executePending(game, cfg);
    expect(played).toBe(game);
  });

  it("will not plan a turn once the game is over", async () => {
    const cfg = config(both());
    let game = startGame(setup());
    let guard = 0;
    while (!game.over && guard < 60) {
      game = await advanceTurn(game, cfg);
      guard += 1;
    }
    const after = await planNextTurn(game, cfg);
    expect(after.pending).toBeNull();
    expect(after).toBe(game);
  }, 60_000);
});

describe("the LLM prompt", () => {
  const request = () =>
    ordersRequestFor(setup(), "blue", config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    }));

  it("states the side, the turn and the reply format", () => {
    const prompt = buildOrdersPrompt(request());
    expect(prompt).toContain("You command BLUE");
    expect(prompt).toContain("TURN 1");
    expect(prompt).toContain("REPLY FORMAT");
    expect(prompt).toContain("optionId");
  });

  it("lists every option by its exact id", () => {
    // A model can only choose an id it was shown, so every legal option has
    // to be in the prompt or it is effectively illegal.
    const req = request();
    const prompt = buildOrdersPrompt(req);
    for (const options of Object.values(req.optionsByElement)) {
      for (const option of options) expect(prompt).toContain(option.id);
    }
  });

  it("includes the directive when given", () => {
    expect(buildOrdersPrompt(request(), "Be aggressive.")).toContain("Be aggressive.");
  });

  it("does not leak the enemy order of battle", () => {
    // The prompt is built from the fog-of-war projection. On turn one nothing
    // is sighted, so no red element id may appear anywhere in it.
    const prompt = buildOrdersPrompt(request());
    expect(prompt).not.toContain("red-1");
    expect(prompt).not.toContain("red-hq");
  });

  it("states the command budget", () => {
    const req = { ...request(), activationBudget: 3 };
    expect(buildOrdersPrompt(req)).toContain("COMMAND CAPACITY: 3 element(s)");
  });

  it("says what happens to orders beyond the budget", () => {
    // ⚠ THE PROMPT USED TO STATE THE CAP AND NOT THE PENALTY. `validateOrders`
    // drops surplus orders from the END of the list, which was true and
    // undocumented — so a model that led with context and finished with its
    // decisive move lost precisely the order it cared most about, and the
    // truncation was effectively random with respect to importance.
    const prompt = buildOrdersPrompt({ ...request(), activationBudget: 3 });
    expect(prompt).toContain("MOST IMPORTANT FIRST");
    expect(prompt).toContain("DROPPED FROM THE END");
  });

  it("does not threaten a budget that is not being enforced", () => {
    // With the module off the budget is Infinity and nothing is refused, so
    // an overrun warning would be a rule the player cannot break.
    const prompt = buildOrdersPrompt(request());
    expect(prompt).toContain("You may order every element.");
    expect(prompt).not.toContain("DROPPED FROM THE END");
  });
});

describe("reading a model's reply", () => {
  it("parses clean JSON", () => {
    const parsed = parseOrdersReply('{"plan":"push left","orders":[{"actorId":"a","optionId":"b","why":"c"}]}');
    expect(parsed.plan).toBe("push left");
    expect(parsed.intents).toEqual([{ actorId: "a", optionId: "b", rationale: "c" }]);
  });

  it("parses JSON wrapped in a code fence", () => {
    // Models do this constantly and it is not worth losing a turn over.
    const parsed = parseOrdersReply('```json\n{"orders":[{"actorId":"a","optionId":"b"}]}\n```');
    expect(parsed.intents).toHaveLength(1);
  });

  it("parses JSON with prose in front of it", () => {
    const parsed = parseOrdersReply('Sure! Here you go:\n{"orders":[{"actorId":"a","optionId":"b"}]}');
    expect(parsed.intents).toHaveLength(1);
  });

  it("parses a bare array", () => {
    expect(parseOrdersReply('[{"actorId":"a","optionId":"b"}]').intents).toHaveLength(1);
  });

  it("reports an unreadable reply rather than guessing", () => {
    // A parser that invented an order would make a broken model look like a
    // bad commander.
    expect(parseOrdersReply("I think we should attack!").parseError).toBeTruthy();
    expect(parseOrdersReply("").parseError).toBeTruthy();
    expect(parseOrdersReply("{not json").parseError).toBeTruthy();
  });

  it("skips malformed entries but keeps good ones", () => {
    const parsed = parseOrdersReply(
      '{"orders":[{"actorId":"a","optionId":"b"},{"actorId":"c"},{"nonsense":true}]}',
    );
    expect(parsed.intents).toHaveLength(1);
  });
});

describe("the LLM commander when things go wrong", () => {
  const request = () =>
    ordersRequestFor(setup(), "blue", config({
      blue: heuristicOrdersCommander("blue"),
      red: heuristicOrdersCommander("red"),
    }));

  it("holds position and says why when the model is unreachable", async () => {
    // A proxy 403 must not throw away an engagement in progress.
    const commander = llmCommander({ side: "blue", call: unavailableModelCall });
    const orders = await commander.planTurn(request());
    expect(orders.intents).toEqual([]);
    expect(orders.plan).toMatch(/unreachable/);
  });

  it("holds position and says why when the reply is unreadable", async () => {
    const commander = llmCommander({ side: "blue", call: async () => "no." });
    const orders = await commander.planTurn(request());
    expect(orders.intents).toEqual([]);
    expect(orders.plan).toMatch(/unreadable/);
  });

  it("passes through what it could read", async () => {
    const req = request();
    const [actorId, options] = Object.entries(req.optionsByElement)[0];
    const commander = llmCommander({
      side: "blue",
      call: async () =>
        `{"plan":"probe","orders":[{"actorId":"${actorId}","optionId":"${options[0].id}"}]}`,
    });
    const orders = await commander.planTurn(req);
    expect(orders.plan).toBe("probe");
    expect(validateOrders(orders, req).accepted).toHaveLength(1);
  });

  it("is declared as an llm, so the log records who decided", async () => {
    expect(llmCommander({ side: "red", call: unavailableModelCall }).kind).toBe("llm");
  });
});
