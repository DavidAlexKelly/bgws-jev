/**
 * ASPECT: which face of a vehicle a shot arrives at.
 *
 * `flank` has been a declared DRM since the table was written, and the wiring
 * guard excused it as "needs a commander that manoeuvres for aspect rather
 * than firing frontally". That was a misdiagnosis of exactly the kind this
 * project keeps catching: there was no facing on a Force Element and no code
 * computing aspect, so the cleverest commander alive would not have made it
 * fire once. It was a wiring gap wearing a commander-behaviour excuse.
 *
 * The rule it now implements: you face where you last moved, and giving away
 * your flank is the price of manoeuvring in front of someone.
 */

import { describe, expect, it } from "vitest";

import { bearingDeg } from "../lib/board";
import { flatTerrain } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import { createRng } from "./dice";
import { EventLog, type ResolutionEvent } from "./events";
import { HOUSE_V1 } from "./ruleset";
import { resolveAction, type PhaseConfig } from "./turnLoop";

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

function board(elements: ForceElement[]): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  for (const element of elements) {
    const viewer: Side = element.side === "blue" ? "red" : "blue";
    sighting[viewer][element.id] = "full";
  }
  return {
    gameId: "aspect",
    scenarioId: "aspect",
    turn: 1,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((f) => [f.id, f])),
    sighting,
    rng: { seed: "aspect", cursor: 0 },
  };
}

function phaseConfig(seed = "aspect"): PhaseConfig {
  return {
    ruleset: HOUSE_V1,
    terrain: flatTerrain(),
    rng: createRng(seed),
    log: new EventLog(),
    maxTurns: 40,
  };
}

const fired = (log: EventLog) =>
  log.all().find((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "directFire")!;

/**
 * Red sits at the origin. Blue is due north of it and shoots south.
 * `redFacing` is set by moving red before the shot.
 */
function shootSouthAt(redFacing: "north" | "south" | "none") {
  const origin = { lat: 54.71, lng: 20.51 };
  const state = board([
    fe({ id: "B1", side: "blue", position: { lat: 54.715, lng: 20.51 } }),
    fe({ id: "R1", side: "red", position: origin }),
  ]);
  const config = phaseConfig();

  let next = state;
  if (redFacing !== "none") {
    // Red moves, which is the only thing that sets facing.
    const destination =
      redFacing === "north"
        ? { lat: 54.7105, lng: 20.51 } // towards blue
        : { lat: 54.7095, lng: 20.51 }; // away from blue
    next = resolveAction(
      next,
      { id: "m", kind: "move", actorId: "R1", destination, summary: "move" },
      config,
      1,
    );
  }

  next = resolveAction(
    next,
    { id: "f", kind: "fire", actorId: "B1", targetId: "R1", summary: "fire" },
    config,
    1,
  );

  return { state: next, config, shot: fired(config.log) };
}

describe("facing comes from movement", () => {
  it("is the bearing of the move, and nothing else sets it", () => {
    const state = board([fe({ id: "B1", side: "blue" })]);
    const config = phaseConfig();
    expect(state.forceElements.B1.facing).toBeUndefined();

    const destination = { lat: 54.715, lng: 20.51 };
    const next = resolveAction(
      state,
      { id: "m", kind: "move", actorId: "B1", destination, summary: "move" },
      config,
      1,
    );

    expect(next.forceElements.B1.facing).toBeCloseTo(
      bearingDeg(state.forceElements.B1.position, destination),
      0,
    );
  });
});

describe("who gets the flank bonus", () => {
  const hasFlank = (shot: ResolutionEvent) =>
    shot.modifiers.some((modifier) => modifier.source === "flank");

  it("a shot into the back of something that drove away from you", () => {
    // Red moved south; blue is north of it, so blue is looking at its rear.
    expect(hasFlank(shootSouthAt("south").shot)).toBe(true);
  });

  it("not a shot into the front of something driving at you", () => {
    expect(hasFlank(shootSouthAt("north").shot)).toBe(false);
  });

  it("NOT against something that has not moved at all", () => {
    // An element that has not moved is assumed to be oriented on its arc. You
    // cannot flank a stationary tank that has been watching you all along by
    // walking round it — aspect is something the TARGET gives away by
    // manoeuvring, which is the trade the rule exists to create.
    expect(hasFlank(shootSouthAt("none").shot)).toBe(false);
  });
});

describe("combined fire takes the worst aspect among the firers", () => {
  /**
   * Red drives due south. `secondFirer` is where the other blue element
   * stands: due north of red is its rear arc, due south is the front it is
   * driving towards.
   */
  function combinedShotWith(secondFirer: { lat: number; lng: number }): ResolutionEvent {
    const state = board([
      fe({ id: "B1", side: "blue", position: { lat: 54.7115, lng: 20.51 } }),
      fe({ id: "B2", side: "blue", position: secondFirer }),
      fe({ id: "R1", side: "red", position: { lat: 54.71, lng: 20.51 } }),
    ]);
    const config = phaseConfig();

    const moved = resolveAction(
      state,
      {
        id: "m",
        kind: "move",
        actorId: "R1",
        destination: { lat: 54.7095, lng: 20.51 },
        summary: "move",
      },
      config,
      1,
    );

    resolveAction(
      moved,
      {
        id: "c",
        kind: "fire",
        actorId: "B1",
        actorIds: ["B1", "B2"],
        targetId: "R1",
        summary: "combined",
      },
      config,
      1,
    );

    return fired(config.log);
  }

  it("gives it when both are behind the target", () => {
    // The control: without this the test below would pass even if flank never
    // fired for combined shots at all.
    const shot = combinedShotWith({ lat: 54.712, lng: 20.51 });
    expect(shot.actorIds).toEqual(["B1", "B2"]);
    expect(shot.modifiers.some((modifier) => modifier.source === "flank")).toBe(true);
  });

  it("withholds it when one firer is in the target's front arc", () => {
    // 9.2.1.2: "use the most detrimental modifiers to the Firing side". One
    // element in a good position should not launder the whole troop's aspect.
    // B2 is due south, which is exactly where red is driving.
    const shot = combinedShotWith({ lat: 54.708, lng: 20.51 });
    expect(shot.actorIds).toEqual(["B1", "B2"]);
    expect(shot.modifiers.some((modifier) => modifier.source === "flank")).toBe(false);
  });
});
