/**
 * ARC — the Action-Reaction-Counteraction sub-phase (Core Rules 7.0).
 *
 * Every test here cites the rulebook, because ARC was twice implemented from
 * the NAME rather than from the text and both readings were wrong:
 *
 *   1. Only the A existed. Orders executed one at a time and nobody
 *      interfered, so a troop could drive across a loaded gun's frontage at
 *      400 m and take no fire.
 *   2. The C was built as a riposte — the actor shooting back at whoever had
 *      just reacted to it, inside the same activation. "Action, Reaction,
 *      Counteraction" sounds exactly like that. 7.2 is a second ROUND, after
 *      every element has acted, in which reserves move up and anything that
 *      has not yet fired may shoot.
 *
 * So these tests quote the rules they are asserting. A test that only asserts
 * what the code does is a test that will happily lock in the third wrong
 * reading.
 */

import { describe, expect, it } from "vitest";

import { distanceM } from "../lib/board";
import { flatTerrain } from "../lib/lineOfSight";
import { advanceTurn, startGame } from "../lib/liveGame";
import { scenarioFactory } from "../lib/forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import type { ForceElement, GameState, Side } from "../lib/state";
import { heuristicCommander } from "./commander";
import { createRng } from "./dice";
import { EventLog, type ResolutionEvent } from "./events";
import { MEETING_ENGAGEMENT_V1 } from "./forceList";
import { heuristicOrdersCommander, willReact } from "./orders";
import { HOUSE_V1, withModules } from "./ruleset";
import {
  attemptSightingInterrupt,
  counteractionFireOptionsFor,
  mayProceed,
  nominateReserves,
  noStandingOrders,
  reserveMoveOptionsFor,
  resolveAction,
  resolveAssaultAction,
  runCounteractionRound,
  runReactiveFire,
  runTurn,
  type PhaseConfig,
} from "./turnLoop";

const terrain = proceduralTerrain(STANDARD_GROUND);
const ARC_ON = withModules(HOUSE_V1, { reactionFire: true, counteraction: true });
const REACTION_ONLY = withModules(HOUSE_V1, { reactionFire: true });
const COUNTERACTION_ONLY = withModules(HOUSE_V1, { counteraction: true });

// ── Fixtures ───────────────────────────────────────────────────────────────

function fe(overrides: Partial<ForceElement> & { id: string; side: Side }): ForceElement {
  return {
    label: overrides.id,
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
    position: { lat: 54.71, lng: 20.51 },
    ...overrides,
  };
}

/** A board where both sides can see each other, about 1.2 km apart. */
function board(elements: ForceElement[]): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  for (const element of elements) {
    const viewer: Side = element.side === "blue" ? "red" : "blue";
    sighting[viewer][element.id] = "full";
  }
  return {
    gameId: "arc",
    scenarioId: "arc",
    turn: 1,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((f) => [f.id, f])),
    sighting,
    rng: { seed: "arc", cursor: 0 },
  };
}

function phaseConfig(ruleset = ARC_ON, seed = "arc"): PhaseConfig {
  return {
    ruleset,
    terrain: flatTerrain(),
    rng: createRng(seed),
    log: new EventLog(),
    maxTurns: 40,
  };
}

/** A whole game through the ORDERS sequence (the play screen and the models). */
async function playOrders(ruleset: typeof HOUSE_V1, seed: string) {
  const log = new EventLog();
  let game = startGame(scenarioFactory(MEETING_ENGAGEMENT_V1, ruleset)());
  const config = {
    ruleset,
    terrain,
    commanders: { blue: heuristicOrdersCommander("blue"), red: heuristicOrdersCommander("red") },
    rng: createRng(`${seed}:dice`),
    log,
    maxTurns: 40,
  };
  while (!game.over) game = await advanceTurn(game, config);
  return {
    game,
    resolutions: log.all().filter((e): e is ResolutionEvent => e.type === "resolution"),
  };
}

/** One turn through the ACTIVATION sequence (the one the harness measures). */
async function playActivation(ruleset: typeof HOUSE_V1, seed: string, turns = 6) {
  const log = new EventLog();
  const rng = createRng(`${seed}:dice`);
  let state = scenarioFactory(MEETING_ENGAGEMENT_V1, ruleset)();
  const config = {
    ruleset,
    terrain,
    commanders: { blue: heuristicCommander("blue", rng), red: heuristicCommander("red", rng) },
    rng,
    log,
    maxTurns: 40,
  };
  for (let i = 0; i < turns; i += 1) state = await runTurn(state, config);
  return {
    state,
    resolutions: log.all().filter((e): e is ResolutionEvent => e.type === "resolution"),
  };
}

// ── 7.1.3 Reactive Fire: the R ─────────────────────────────────────────────

describe("standing orders decide whether an element answers (our stand-in for 7.1.3)", () => {
  // The rulebook gates Reactive Fire on holding one of eleven Order Verbs AND
  // the target moving through a TAI assigned to that FE. We model neither, so
  // rules of engagement stand in for both — same decision, same moment, same
  // binding period. See ReactionRule.
  const ctx = (rangeM: number, wasFiredUpon = false) => ({
    rangeM,
    shortRangeM: 1000,
    wasFiredUpon,
  });

  it("never holds fire whatever happens", () => {
    expect(willReact({ actorId: "a", engage: "never" }, ARC_ON, ctx(100, true))).toBe(false);
  });

  it("always answers anything in reach", () => {
    expect(willReact({ actorId: "a", engage: "always" }, ARC_ON, ctx(2900))).toBe(true);
  });

  it("withinShortRange is the ambush: quiet at distance, deadly up close", () => {
    const order = { actorId: "a", engage: "withinShortRange" as const };
    expect(willReact(order, ARC_ON, ctx(1500))).toBe(false);
    expect(willReact(order, ARC_ON, ctx(600))).toBe(true);
  });

  it("ifFiredUpon lets armour drive past until someone shoots", () => {
    const order = { actorId: "a", engage: "ifFiredUpon" as const };
    expect(willReact(order, ARC_ON, ctx(300, false))).toBe(false);
    expect(willReact(order, ARC_ON, ctx(300, true))).toBe(true);
  });

  it("withinM tightens whatever the rule would otherwise allow", () => {
    const order = { actorId: "a", engage: "always" as const, withinM: 500 };
    expect(willReact(order, ARC_ON, ctx(400))).toBe(true);
    expect(willReact(order, ARC_ON, ctx(600))).toBe(false);
  });

  it("an element with no standing order uses the ruleset default", () => {
    // Defaulting to "never" would make the mechanic invisible unless every
    // commander remembered to set it, and a model that forgets would be
    // indistinguishable from a mechanic that does nothing.
    expect(ARC_ON.reaction.defaultEngage).toBe("withinShortRange");
    expect(willReact(undefined, ARC_ON, ctx(600))).toBe(true);
    expect(willReact(undefined, ARC_ON, ctx(1500))).toBe(false);
  });
});

describe("who may Reactive Fire (7.1.3)", () => {
  const mover = () => fe({ id: "B1", side: "blue", position: { lat: 54.71, lng: 20.51 } });
  const watcher = (markers: ForceElement["markers"] = []) =>
    fe({ id: "R1", side: "red", position: { lat: 54.719, lng: 20.51 }, markers });

  function reactionsIn(log: EventLog): ResolutionEvent[] {
    return log
      .all()
      .filter((e): e is ResolutionEvent => e.type === "resolution" && e.phase === "arcReaction");
  }

  it("answers an enemy that is moving", () => {
    const config = phaseConfig();
    const state = board([mover(), watcher()]);
    runReactiveFire(state, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    expect(reactionsIn(config.log).length).toBe(1);
  });

  it("does nothing when the module is off", () => {
    const config = phaseConfig(HOUSE_V1);
    const state = board([mover(), watcher()]);
    runReactiveFire(state, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    expect(reactionsIn(config.log).length).toBe(0);
  });

  it("may not be taken by an element that has already Activated", () => {
    // "If it is the Action-Reaction Round, the reacting FE has not yet
    // Activated, or has been given a Hold Action." Overwatch is something you
    // are doing instead of acting, not as well as.
    const config = phaseConfig();
    const state = board([mover(), watcher(["activated"])]);
    runReactiveFire(state, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    expect(reactionsIn(config.log).length).toBe(0);
  });

  it("MAY be taken by an element that activated and then Held", () => {
    // "...or has been given a Hold Action." Holding is the overwatch order.
    const config = phaseConfig();
    const state = board([mover(), watcher(["activated", "held"])]);
    runReactiveFire(state, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    expect(reactionsIn(config.log).length).toBe(1);
  });

  it("in the Counteraction Round the test is the FIRED marker instead", () => {
    // "If it is the Counteraction Round, it did not Fire in the
    // Action-Reaction round (i.e. it does not have a FIRED marker)." An
    // element that moved in the first round may still be watching in the
    // second, which the Action-Reaction test would have excluded.
    const activated = phaseConfig();
    runReactiveFire(
      board([mover(), watcher(["activated", "moved"])]),
      "B1",
      "blue",
      activated,
      1,
      noStandingOrders(),
      "counteraction",
    );
    expect(reactionsIn(activated.log).length).toBe(1);

    const fired = phaseConfig();
    runReactiveFire(
      board([mover(), watcher(["fired"])]),
      "B1",
      "blue",
      fired,
      1,
      noStandingOrders(),
      "counteraction",
    );
    expect(reactionsIn(fired.log).length).toBe(0);
  });

  it("costs the reactor the rest of its turn", () => {
    // "The FE/Group that has Reactive Fired cannot take any further Action for
    // the remainder of the Turn." Carried by the FIRED marker, which every
    // other part of the loop already gates on.
    const config = phaseConfig();
    const next = runReactiveFire(
      board([mover(), watcher()]),
      "B1",
      "blue",
      config,
      1,
      noStandingOrders(),
      "actionReaction",
    );
    expect(next.forceElements.R1.markers).toContain("fired");
    expect(next.forceElements.R1.markers).toContain("reacted");
  });

  it("cannot be taken twice by the same element in a turn", () => {
    const config = phaseConfig();
    let state = board([mover(), watcher()]);
    state = runReactiveFire(state, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    state = runReactiveFire(state, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    expect(reactionsIn(config.log).length).toBe(1);
  });

  it("cannot answer something its side has not sighted", () => {
    const config = phaseConfig();
    const state = board([mover(), watcher()]);
    const blind = { ...state, sighting: { blue: state.sighting.blue, red: {} } };
    runReactiveFire(blind, "B1", "blue", config, 1, noStandingOrders(), "actionReaction");
    expect(reactionsIn(config.log).length).toBe(0);
  });

  it("caps how many elements answer one action", () => {
    // HOUSE, and stricter than the rulebook, which allows any number. See
    // ReactionRule.maxReactorsPerAction for why a bot needs the cap and a
    // human does not.
    const config = phaseConfig();
    const watchers = [1, 2, 3, 4].map((n) =>
      fe({ id: `R${n}`, side: "red", position: { lat: 54.719, lng: 20.51 } }),
    );
    runReactiveFire(
      board([mover(), ...watchers]),
      "B1",
      "blue",
      config,
      1,
      noStandingOrders(),
      "actionReaction",
    );
    expect(reactionsIn(config.log).length).toBe(ARC_ON.reaction.maxReactorsPerAction);
  });

  it("carries the snap-shot penalty on every answering shot", () => {
    const config = phaseConfig();
    runReactiveFire(
      board([mover(), watcher()]),
      "B1",
      "blue",
      config,
      1,
      noStandingOrders(),
      "actionReaction",
    );
    for (const event of reactionsIn(config.log)) {
      expect(event.modifiers.some((m) => m.source === "snapShot")).toBe(true);
    }
    // Ours, not the rulebook's — the rulebook charges no DRM and makes the
    // reactor forfeit its turn instead. We do both; see ReactionRule.
    expect(ARC_ON.reaction.snapShotDrm).toBeLessThan(0);
  });
});

describe("a reaction can stop the move it interrupted (7.1.3)", () => {
  it("mayProceed is false once the mover is Disrupted, Broken or gone", () => {
    // "the moving FE may continue its movement (unless it has become
    // Disrupted or Broken)".
    const state = board([
      fe({ id: "A", side: "blue", morale: "suppressed1" }),
      fe({ id: "B", side: "blue", morale: "disrupted" }),
      fe({ id: "C", side: "blue", morale: "broken" }),
      fe({ id: "D", side: "blue", combatStrength: 0 }),
    ]);
    expect(mayProceed(state, "A")).toBe(true);
    expect(mayProceed(state, "B")).toBe(false);
    expect(mayProceed(state, "C")).toBe(false);
    expect(mayProceed(state, "D")).toBe(false);
  });

  it("an answered advance is sometimes halted outright", () => {
    // The whole tactical point of overwatch, and the thing that was missing
    // while reactions resolved AFTER the mover had already arrived. Asserted
    // over seeds rather than one roll because whether a given snap shot
    // suppresses is a die roll — the claim is that it CAN happen, not that it
    // always does.
    let halted = 0;
    for (let seed = 0; seed < 30; seed += 1) {
      const config = phaseConfig(ARC_ON, `halt${seed}`);
      const state = board([
        // Already shaken, so one more step takes it out of Disrupted.
        fe({ id: "B1", side: "blue", morale: "suppressed2" }),
        fe({ id: "R1", side: "red", position: { lat: 54.719, lng: 20.51 }, combatStrength: 16 }),
      ]);
      const next = runReactiveFire(
        state,
        "B1",
        "blue",
        config,
        1,
        noStandingOrders(),
        "actionReaction",
      );
      if (!mayProceed(next, "B1")) halted += 1;
    }
    expect(halted).toBeGreaterThan(0);
  });
});

// ── 2.1.13 Reserves ────────────────────────────────────────────────────────

describe("reserves (2.1.13)", () => {
  const six = () =>
    board([
      ...[1, 2, 3, 4, 5, 6].map((n) =>
        fe({ id: `B${n}`, side: "blue", position: { lat: 54.7 + n / 1000, lng: 20.51 } }),
      ),
      fe({ id: "R1", side: "red", position: { lat: 54.73, lng: 20.51 } }),
    ]);

  const reserved = (state: GameState) =>
    Object.values(state.forceElements).filter((f) => f.markers.includes("reserve"));

  it("holds back at most one third of a side", () => {
    // "Only one-third of FE/Groups in a side may be given a Reserve Order."
    const next = nominateReserves(six(), "blue", phaseConfig(), [
      "B1",
      "B2",
      "B3",
      "B4",
      "B5",
      "B6",
    ]);
    expect(reserved(next).length).toBe(2);
  });

  it("nominates nobody when the Counteraction Round is switched off", () => {
    const next = nominateReserves(six(), "blue", phaseConfig(HOUSE_V1), ["B1", "B2"]);
    expect(reserved(next).length).toBe(0);
  });

  it("takes the rearmost when no commander said", () => {
    // The activation sequence has no orders step in which to nominate. A
    // default of "nobody" would leave the Counteraction Round's first stage
    // permanently empty and make the module look inert for a reason that has
    // nothing to do with the rule.
    const next = nominateReserves(six(), "blue", phaseConfig());
    const ids = reserved(next).map((f) => f.id).sort();
    // Red is to the north, so the lowest-numbered (southernmost) are rearmost.
    expect(ids).toEqual(["B1", "B2"]);
  });

  it("ignores elements the commander does not own or that do not exist", () => {
    const next = nominateReserves(six(), "blue", phaseConfig(), ["R1", "nonsense", "B4"]);
    expect(reserved(next).map((f) => f.id)).toEqual(["B4"]);
  });
});

describe("reserve movement (7.2.1)", () => {
  const reserveAndEnemy = (markers: ForceElement["markers"]) =>
    board([
      fe({ id: "B1", side: "blue", markers, position: { lat: 54.7, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: { lat: 54.73, lng: 20.51 } }),
    ]);

  it("is offered to a reserve that has already moved", () => {
    // "They can do this even if they have a MOVED marker." The extra move is
    // the entire privilege of being held back.
    const state = reserveAndEnemy(["reserve", "moved"]);
    const options = reserveMoveOptionsFor(state, state.forceElements.B1, phaseConfig());
    expect(options.length).toBeGreaterThan(0);
  });

  it("is not offered to anything that is not in reserve", () => {
    const state = reserveAndEnemy([]);
    expect(reserveMoveOptionsFor(state, state.forceElements.B1, phaseConfig())).toEqual([]);
  });

  it("is not offered to a reserve that has fired", () => {
    // "...only be taken by FEs with a Reserve Order and no FIRED marker."
    const state = reserveAndEnemy(["reserve", "fired"]);
    expect(reserveMoveOptionsFor(state, state.forceElements.B1, phaseConfig())).toEqual([]);
  });

  it("is not offered twice", () => {
    const state = reserveAndEnemy(["reserve", "reserveMoved"]);
    expect(reserveMoveOptionsFor(state, state.forceElements.B1, phaseConfig())).toEqual([]);
  });

  it("moves no further than the ruleset's reserve distance", () => {
    const state = reserveAndEnemy(["reserve"]);
    const [option] = reserveMoveOptionsFor(state, state.forceElements.B1, phaseConfig());
    const from = state.forceElements.B1.position;
    // 0.03 degrees of latitude is well over 3 km, so the cap must bite.
    // Measured with the game's own distanceM: a hand-rolled approximation
    // disagreed with it by a metre and failed the test on the arithmetic
    // rather than on the rule.
    const metres = distanceM(from, option.destination!);
    // A millimetre of tolerance, because interpolating a fraction of a great
    // circle and then measuring it back lands 1.4e-10 m over the cap. That is
    // IEEE754, not a rules violation.
    expect(metres).toBeLessThanOrEqual(ARC_ON.counteraction.reserveMoveM + 0.001);
  });
});

// ── 7.2.2 Counteraction Fire ───────────────────────────────────────────────

describe("counteraction fire (7.2.2)", () => {
  const pair = (markers: ForceElement["markers"]) =>
    board([
      fe({ id: "B1", side: "blue", markers }),
      fe({ id: "R1", side: "red", position: { lat: 54.719, lng: 20.51 } }),
    ]);

  it("is offered to anything that has not fired", () => {
    // "Any FE/Group that does not have a FIRED marker (i.e. that did not Fire
    // in the Action-Reaction Round) can DirF in the Counteraction Round."
    const state = pair([]);
    expect(counteractionFireOptionsFor(state, state.forceElements.B1, phaseConfig()).length)
      .toBeGreaterThan(0);
  });

  it("is offered to something that MOVED but held its fire", () => {
    // Note what is not required: the element need not be unactivated. This is
    // the manoeuvre the round exists to permit — move up in the first round,
    // shoot in the second.
    const state = pair(["activated", "moved"]);
    expect(counteractionFireOptionsFor(state, state.forceElements.B1, phaseConfig()).length)
      .toBeGreaterThan(0);
  });

  it("is not offered to anything that has fired", () => {
    const state = pair(["fired"]);
    expect(counteractionFireOptionsFor(state, state.forceElements.B1, phaseConfig())).toEqual([]);
  });

  it("is not offered twice", () => {
    const state = pair(["counteracted"]);
    expect(counteractionFireOptionsFor(state, state.forceElements.B1, phaseConfig())).toEqual([]);
  });

  it("carries the Counteraction Round penalty", () => {
    // "note that DRM penalties for Firing in the Counteraction Round apply".
    // The figure is ours — Player Aid 4 is not in the box we have — but the
    // penalty's existence is the rulebook's, and without it there would be no
    // reason to fire in the first round at all.
    expect(ARC_ON.counteraction.fireDrm).toBeLessThan(0);
  });
});

describe("the Counteraction Round as a round", () => {
  function twoOnTwo(): GameState {
    return board([
      fe({ id: "B1", side: "blue" }),
      fe({ id: "B2", side: "blue" }),
      fe({ id: "R1", side: "red", position: { lat: 54.719, lng: 20.51 } }),
      fe({ id: "R2", side: "red", position: { lat: 54.719, lng: 20.511 } }),
    ]);
  }

  it("passing is final for the turn", async () => {
    // "A side that has passed cannot at a later point, after seeing the other
    // side's DirFs, then declare that it wishes to DirF. Passing is final for
    // the Turn." Without this the round is not a decision, it is a mop-up.
    const config = phaseConfig();
    let asked = 0;
    const next = await runCounteractionRound(
      twoOnTwo(),
      config,
      1,
      "blue",
      noStandingOrders(),
      async (_state, side, options) => {
        if (side === "blue") {
          asked += 1;
          return null; // Pass, immediately and for good.
        }
        return options[0];
      },
    );

    expect(asked).toBe(1);
    expect(next.forceElements.B1.markers).not.toContain("counteracted");
    expect(next.forceElements.B2.markers).not.toContain("counteracted");
    // Red, which did not pass, got its shots in.
    const red = [next.forceElements.R1, next.forceElements.R2];
    expect(red.some((f) => f.markers.includes("counteracted"))).toBe(true);
  });

  it("does nothing at all when the module is off", async () => {
    const config = phaseConfig(HOUSE_V1);
    const before = twoOnTwo();
    const next = await runCounteractionRound(
      before,
      config,
      1,
      "blue",
      noStandingOrders(),
      async (_state, _side, options) => options[0],
    );
    expect(next).toBe(before);
    expect(config.log.all().length).toBe(0);
  });

  it("terminates rather than looping on the same element", async () => {
    // Every stage has to consume something each pass or the round never ends.
    const config = phaseConfig();
    const next = await runCounteractionRound(
      twoOnTwo(),
      config,
      1,
      "blue",
      noStandingOrders(),
      async (_state, _side, options) => options[0],
    );
    for (const id of ["B1", "B2", "R1", "R2"]) {
      const element = next.forceElements[id];
      if (element.combatStrength > 0) expect(element.markers).toContain("counteracted");
    }
  });
});

// ── 10.0 Attempt Sighting, as an interrupt ─────────────────────────────────

describe("attempt sighting against an activating element (10.0, 7.1)", () => {
  const CONCEALED = withModules(HOUSE_V1, { concealment: true });

  const hidden = (overrides: Partial<ForceElement> = {}) =>
    fe({ id: "B1", side: "blue", concealed: true, ...overrides });
  const looker = (overrides: Partial<ForceElement> = {}) =>
    fe({ id: "R1", side: "red", position: { lat: 54.711, lng: 20.51 }, ...overrides });

  /** Nobody has sighted anybody yet — the interrupt is what changes that. */
  function unseen(elements: ForceElement[]): GameState {
    return { ...board(elements), sighting: { blue: {}, red: {} } };
  }

  function sightingsIn(log: EventLog) {
    return log
      .all()
      .filter((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "sighting");
  }

  it("is attempted when a concealed element activates", () => {
    const config = phaseConfig(CONCEALED);
    attemptSightingInterrupt(unseen([hidden(), looker()]), "B1", "blue", config, 1);
    expect(sightingsIn(config.log).length).toBe(1);
  });

  it("is not attempted against something already on the map", () => {
    // There is nothing to reveal about an element that is not Concealed.
    const config = phaseConfig(CONCEALED);
    attemptSightingInterrupt(
      unseen([hidden({ concealed: false }), looker()]),
      "B1",
      "blue",
      config,
      1,
    );
    expect(sightingsIn(config.log).length).toBe(0);
  });

  it("does nothing when the concealment module is off", () => {
    const config = phaseConfig(HOUSE_V1);
    attemptSightingInterrupt(unseen([hidden(), looker()]), "B1", "blue", config, 1);
    expect(sightingsIn(config.log).length).toBe(0);
  });

  it("is ONE observer, not the whole side", () => {
    // "Any one non-Activating FE can make an Attempt Sighting Action." Four
    // pairs of eyes do not get four rolls; that would make concealment
    // worthless against any force of reasonable size.
    const config = phaseConfig(CONCEALED);
    const watchers = [1, 2, 3, 4].map((n) =>
      fe({ id: `R${n}`, side: "red", position: { lat: 54.711, lng: 20.51 } }),
    );
    attemptSightingInterrupt(unseen([hidden(), ...watchers]), "B1", "blue", config, 1);
    expect(sightingsIn(config.log).length).toBe(1);
  });

  it("flips the counter on a full sighting, and not on a partial one", () => {
    // 2.1.15: "Concealment is removed ... if the FE is revealed through an
    // Attempt Sighting." A partial contact is knowing something is there, not
    // knowing what it is.
    //
    // `partialSighting` has to be on for this test to mean anything: with it
    // off, resolveSighting folds every partial into a full and the "does not
    // flip" branch below could never be exercised.
    const THREE_STATE = withModules(HOUSE_V1, { concealment: true, partialSighting: true });
    let flipped = 0;
    let partials = 0;
    for (let seed = 0; seed < 40; seed += 1) {
      const config = phaseConfig(THREE_STATE, `sight${seed}`);
      const next = attemptSightingInterrupt(
        unseen([hidden(), looker()]),
        "B1",
        "blue",
        config,
        1,
      );
      const [event] = sightingsIn(config.log);
      if (event.result === "full") {
        expect(next.forceElements.B1.concealed).toBe(false);
        flipped += 1;
      } else {
        expect(next.forceElements.B1.concealed).toBe(true);
        if (event.result === "partial") partials += 1;
      }
    }
    // Both branches have to be exercised or the assertion above is vacuous.
    expect(flipped).toBeGreaterThan(0);
    expect(partials).toBeGreaterThan(0);
  });

  it("firing flips the counter too", () => {
    // 2.1.15: "Concealment is removed ... if the FE Fires or Assaults."
    // Nothing flipped a counter before this, so concealment was a starting
    // condition that survived a whole game of shooting.
    const config = phaseConfig(CONCEALED);
    const state = board([
      fe({ id: "B1", side: "blue", concealed: true }),
      fe({ id: "R1", side: "red", position: { lat: 54.719, lng: 20.51 } }),
    ]);
    const next = resolveAction(
      state,
      { id: "x", kind: "fire", actorId: "B1", targetId: "R1", summary: "fire" },
      config,
      1,
    );
    expect(next.forceElements.B1.concealed).toBe(false);
  });
});

// ── 9.3.1-9.3.2 Surprise and Defensive Fire ────────────────────────────────

describe("an assault has to go in through fire (9.3.1, 9.3.2)", () => {
  const DEFENSIVE = withModules(HOUSE_V1, { defensiveFire: true });
  const DEFENSIVE_AND_REACTION = withModules(HOUSE_V1, {
    defensiveFire: true,
    reactionFire: true,
  });

  /** An attacker in contact, its target, and a second defender beside it. */
  function objective(): GameState {
    return board([
      fe({ id: "B1", side: "blue", position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: { lat: 54.7101, lng: 20.51 } }),
      fe({ id: "R2", side: "red", position: { lat: 54.7102, lng: 20.51 } }),
      // Well outside the 250 m radius: this one is not in the assault.
      fe({ id: "R3", side: "red", position: { lat: 54.716, lng: 20.51 } }),
    ]);
  }

  const assault = { id: "a", kind: "assault" as const, actorId: "B1", targetId: "R1", summary: "assault" };

  function events(log: EventLog, table: string) {
    return log
      .all()
      .filter((e): e is ResolutionEvent => e.type === "resolution" && e.table === table);
  }

  function defensiveShots(log: EventLog) {
    return log
      .all()
      .filter(
        (e): e is ResolutionEvent =>
          e.type === "resolution" && e.modifiers.some((m) => m.source === "defensiveFire"),
      );
  }

  it("rolls for surprise on every assault", () => {
    // 9.3.4 step 2, and the roll happens whether or not anyone is concealed —
    // which is why excusing `attackerSurprise` as "needs a commander that
    // assaults out of concealment" was the wrong diagnosis for three months.
    const config = phaseConfig(DEFENSIVE);
    resolveAssaultAction(objective(), assault, config, 1, noStandingOrders(), "actionReaction");
    expect(events(config.log, "assault:surprise").length).toBe(1);
  });

  it("does not roll, or let anyone defend, when the module is off", () => {
    const config = phaseConfig(HOUSE_V1);
    resolveAssaultAction(objective(), assault, config, 1, noStandingOrders(), "actionReaction");
    expect(events(config.log, "assault:surprise").length).toBe(0);
    expect(defensiveShots(config.log).length).toBe(0);
  });

  it("lets the defenders shoot back when there is no surprise", () => {
    // Both R1 and R2 are within 250 m of the location, so both are in the
    // assault and both may Defensive Fire. Asserted across seeds because
    // surprise is a die roll and suppresses the whole step.
    let sawDefensiveFire = false;
    let sawSurpriseSilence = false;

    for (let seed = 0; seed < 20; seed += 1) {
      const config = phaseConfig(DEFENSIVE, `def${seed}`);
      resolveAssaultAction(objective(), assault, config, 1, noStandingOrders(), "actionReaction");
      const [surprise] = events(config.log, "assault:surprise");
      const shots = defensiveShots(config.log);

      if (surprise.result === "surprise") {
        // 9.3.1: "enemy FEs may not Reactive Fire or Defensive Fire".
        expect(shots.length).toBe(0);
        sawSurpriseSilence = true;
      } else if (shots.length > 0) {
        sawDefensiveFire = true;
        // "Combined Fire is not possible" — each defender is resolved alone.
        for (const shot of shots) expect(shot.actorIds.length).toBe(1);
      }
    }

    expect(sawDefensiveFire).toBe(true);
    expect(sawSurpriseSilence).toBe(true);
  });

  it("carries the rulebook's own -2", () => {
    // 9.3.2: "resolved as an individual DirF (with a -2 DRM)". One of the few
    // numbers in this ruleset that is quoted rather than invented.
    expect(DEFENSIVE.assault.defensiveFireDrm).toBe(-2);

    for (let seed = 0; seed < 20; seed += 1) {
      const config = phaseConfig(DEFENSIVE, `drm${seed}`);
      resolveAssaultAction(objective(), assault, config, 1, noStandingOrders(), "actionReaction");
      for (const shot of defensiveShots(config.log)) {
        const modifier = shot.modifiers.find((m) => m.source === "defensiveFire");
        expect(modifier?.value).toBe(-2);
      }
    }
  });

  it("defenders in the assault do not ALSO get to react", () => {
    // 9.3.2 splits the defending side in two: those within 250 m defend,
    // everyone else reactive fires. Getting both would double the fire an
    // assault walks into.
    for (let seed = 0; seed < 20; seed += 1) {
      const config = phaseConfig(DEFENSIVE_AND_REACTION, `split${seed}`);
      resolveAssaultAction(objective(), assault, config, 1, noStandingOrders(), "actionReaction");
      const reactions = config.log
        .all()
        .filter((e): e is ResolutionEvent => e.type === "resolution" && e.phase === "arcReaction");
      for (const reaction of reactions) {
        // Only R3, the one outside the radius, may be a reactor.
        expect(reaction.actorIds).toEqual(["R3"]);
      }
    }
  });

  it("draws in every defender within 250 m, not just the one aimed at", () => {
    // 9.3: "Any enemy FE/Group within 250m of the location participates in
    // the Assault as the 'defender'." Attacking into mutually supporting
    // positions should cost more than attacking an isolated one.
    const config = phaseConfig(DEFENSIVE);
    resolveAssaultAction(objective(), assault, config, 1, noStandingOrders(), "actionReaction");
    const [resolution] = config.log
      .all()
      .filter((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "assault")
      .filter((e) => e.table !== "assault:surprise");
    if (resolution) {
      expect(resolution.targetIds).toContain("R1");
      expect(resolution.targetIds).toContain("R2");
      expect(resolution.targetIds).not.toContain("R3");
    }
  });

  it("can stop the assault happening at all", () => {
    // Defensive Fire that Disrupts or Breaks the attacker means no assault:
    // the same rule as a move interrupted by Reactive Fire, and the reason
    // closing with a prepared position is dangerous.
    let stopped = 0;
    for (let seed = 0; seed < 40; seed += 1) {
      const config = phaseConfig(DEFENSIVE, `stop${seed}`);
      const state = board([
        fe({ id: "B1", side: "blue", morale: "suppressed2", position: { lat: 54.71, lng: 20.51 } }),
        fe({ id: "R1", side: "red", combatStrength: 16, position: { lat: 54.7101, lng: 20.51 } }),
      ]);
      const next = resolveAssaultAction(
        state,
        assault,
        config,
        1,
        noStandingOrders(),
        "actionReaction",
      );
      const resolved = config.log
        .all()
        .some(
          (e) => e.type === "resolution" && e.kind === "assault" && e.table !== "assault:surprise",
        );
      if (!resolved && next.forceElements.B1.markers.includes("held")) stopped += 1;
    }
    expect(stopped).toBeGreaterThan(0);
  });
});

// ── Both sequences of play ─────────────────────────────────────────────────

describe("ARC happens in BOTH sequences of play", () => {
  // ⚠ THIS IS THE TEST THAT MAKES THE MODULE SWEEP BELIEVABLE.
  //
  // Reaction and counteraction lived only in the orders sequence, and
  // rules/harness.ts measures the OTHER one. So `moduleImpact("reactionFire")`
  // compared two identical games and would have reported "CEREMONY — delete"
  // for a mechanic that decides engagements. A rule that only one sequence can
  // reach is not finished, however well it works where it is.

  it("the activation sequence enters both ARC phases", async () => {
    const { resolutions } = await playActivation(ARC_ON, "both-activation");
    const phases = new Set(resolutions.map((event) => event.phase));
    expect(phases.has("arcReaction")).toBe(true);
    expect(phases.has("arcCounteraction")).toBe(true);
  }, 30_000);

  it("the orders sequence enters both ARC phases", async () => {
    const { resolutions } = await playOrders(ARC_ON, "both-orders");
    const phases = new Set(resolutions.map((event) => event.phase));
    expect(phases.has("arcReaction")).toBe(true);
    expect(phases.has("arcCounteraction")).toBe(true);
  }, 30_000);

  it("each module is separately reachable, so the sweep can tell them apart", async () => {
    // Swept over seeds rather than asserted on one, because whether any given
    // game contains a reaction is a die roll. Pinning it to a single seed made
    // this test fail the day aspect started changing DRMs — a true claim
    // ("the R is reachable with the C off") broken by an unrelated change to
    // how the dice fall.
    const seeds = ["sep0", "sep1", "sep2", "sep3", "sep4", "sep5"];

    const phasesOver = async (ruleset: typeof HOUSE_V1) => {
      const seen = new Set<string>();
      for (const seed of seeds) {
        const { resolutions } = await playOrders(ruleset, seed);
        for (const event of resolutions) seen.add(event.phase);
      }
      return seen;
    };

    const reaction = await phasesOver(REACTION_ONLY);
    expect(reaction.has("arcReaction")).toBe(true);
    expect(reaction.has("arcCounteraction")).toBe(false);

    const counteraction = await phasesOver(COUNTERACTION_ONLY);
    expect(counteraction.has("arcReaction")).toBe(false);
    expect(counteraction.has("arcCounteraction")).toBe(true);
  }, 60_000);

  it("neither phase is entered with both switched off", async () => {
    const { resolutions } = await playOrders(HOUSE_V1, "neither");
    const phases = new Set(resolutions.map((event) => event.phase));
    expect(phases.has("arcReaction")).toBe(false);
    expect(phases.has("arcCounteraction")).toBe(false);
  }, 30_000);

  it("the snap-shot and counteraction modifiers both fire in a real game", async () => {
    const { resolutions } = await playOrders(ARC_ON, "modifiers");
    const sources = new Set(resolutions.flatMap((e) => e.modifiers.map((m) => m.source)));
    expect(sources.has("snapShot")).toBe(true);
    expect(sources.has("counteractionFire")).toBe(true);
  }, 30_000);

  it("changes how the game goes", async () => {
    const off = await playOrders(HOUSE_V1, "arc-effect");
    const on = await playOrders(ARC_ON, "arc-effect");
    const differs =
      off.game.winner !== on.game.winner || off.game.turns.length !== on.game.turns.length;
    expect(differs).toBe(true);
  }, 30_000);
});

describe("the caps that stop a cascade", () => {
  it("no element reacts more than once a turn", async () => {
    const { resolutions } = await playOrders(ARC_ON, "arc-caps");
    const perTurn = new Map<string, Set<string>>();
    for (const event of resolutions) {
      if (event.phase !== "arcReaction") continue;
      const key = String(event.turn);
      const seen = perTurn.get(key) ?? new Set<string>();
      for (const actor of event.actorIds) {
        expect(seen.has(actor), `${actor} reacted twice on turn ${key}`).toBe(false);
        seen.add(actor);
      }
      perTurn.set(key, seen);
    }
  }, 30_000);

  it("terminates rather than ping-ponging", async () => {
    const { game } = await playOrders(ARC_ON, "arc-terminate");
    expect(game.over).toBe(true);
    expect(game.turns.length).toBeLessThanOrEqual(40);
  }, 30_000);
});
