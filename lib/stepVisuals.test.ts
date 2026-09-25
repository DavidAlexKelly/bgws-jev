import { describe, expect, it } from "vitest";

import type { ResolutionEvent } from "../rules/events";
import type { GameState } from "./state";
import {
  describeOutcome,
  describeRoll,
  fireLinesFor,
  outcomeOf,
  pipsFor,
  eventsVisibleTo,
  stepsVisibleTo,
  visibleIdsFor,
  wrecksIn,
} from "./stepVisuals";

function board(): GameState {
  const fe = (id: string, side: "blue" | "red", lat: number) => ({
    id,
    side,
    label: id,
    sidc: "SFGPUCA-------",
    moveType: "T" as const,
    targetClass: "armoured_vehicle" as const,
    capabilities: [],
    troopQuality: 4,
    combatStrength: 8,
    combatStrengthStart: 8,
    morale: "good" as const,
    markers: [],
    concealed: false,
    isDummy: false,
    position: { lat, lng: 20.5 },
  });
  return {
    gameId: "v",
    scenarioId: "v",
    turn: 2,
    phase: "arcAction",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: {
      B1: fe("B1", "blue", 54.7),
      R1: fe("R1", "red", 54.72),
      R2: fe("R2", "red", 54.74),
    },
    sighting: { blue: {}, red: {} },
    rng: { seed: "v", cursor: 0 },
  };
}

function shot(overrides: Partial<ResolutionEvent> = {}): ResolutionEvent {
  return {
    type: "resolution",
    seq: 1,
    turn: 2,
    phase: "arcAction",
    kind: "directFire",
    rulesetId: "house-v3",
    actorIds: ["B1"],
    targetIds: ["R1"],
    roll: { dice: [4, 3], total: 7, cursor: 0 },
    modifiers: [
      { source: "range", value: -2 },
      { source: "targetInCover", value: 0 },
    ],
    total: 5,
    table: "fire",
    result: "suppressed",
    effects: [{ kind: "morale", feId: "R1", to: "suppressed1" }],
    narrative: "B1 fires on R1.",
    ...overrides,
  };
}

const ALL = new Set(["B1", "R1", "R2"]);

describe("what a shot did", () => {
  it("reads the outcome from the EFFECTS, not from the prose", () => {
    // Result strings are written for people and get reworded; effects are what
    // the engine applied. A picture drawn from prose drifts silently.
    expect(outcomeOf(shot())).toBe("suppressed");
    expect(
      outcomeOf(
        shot({
          result: "anything at all",
          effects: [{ kind: "combatStrength", feId: "R1", delta: -3 }],
        }),
      ),
    ).toBe("hit");
    expect(outcomeOf(shot({ result: "hit!", effects: [] }))).toBe("noEffect");
  });

  it("calls it destroyed when the target has nothing left", () => {
    // Read from the RECORDED STATE, not from the effects: the effects say
    // "-8 Combat Strength", and whether that was the last eight points is a
    // fact about the board rather than about the shot.
    const wrecked = board();
    wrecked.forceElements.R1 = { ...wrecked.forceElements.R1, combatStrength: 0 };
    const killing = shot({ effects: [{ kind: "combatStrength", feId: "R1", delta: -8 }] });

    expect(outcomeOf(killing, wrecked, "R1")).toBe("destroyed");
    expect(describeOutcome(killing, wrecked, "R1")).toBe("destroyed");
    // The same shot against a target that survived is a hit, not a kill.
    expect(outcomeOf(killing, board(), "R1")).toBe("hit");
  });

  it("counts hits so the map can say how many", () => {
    expect(
      describeOutcome(
        shot({
          effects: [
            { kind: "combatStrength", feId: "R1", delta: -3 },
            { kind: "combatStrength", feId: "R1", delta: -3 },
          ],
        }),
      ),
    ).toBe("2 hits");
    expect(describeOutcome(shot())).toBe("suppressed");
    expect(describeOutcome(shot({ effects: [] }))).toBe("no effect");
  });
});

describe("drawing the engagement", () => {
  it("draws a line from firer to target", () => {
    const lines = fireLinesFor({ state: board(), events: [shot()] }, ALL);
    expect(lines).toHaveLength(1);
    expect(lines[0].from.lat).toBeCloseTo(54.7, 5);
    expect(lines[0].to.lat).toBeCloseTo(54.72, 5);
    expect(lines[0].side).toBe("blue");
    expect(lines[0].note).toBe("suppressed");
  });

  it("draws one line per firer per target, so a combined shot shows as one", () => {
    const lines = fireLinesFor(
      { state: board(), events: [shot({ actorIds: ["B1"], targetIds: ["R1", "R2"] })] },
      ALL,
    );
    expect(lines).toHaveLength(2);
  });

  it("draws nothing through the fog", () => {
    // Half a line leaking out of the fog tells a player where an unsighted
    // enemy is, which is exactly what the fog is for.
    const lines = fireLinesFor({ state: board(), events: [shot()] }, new Set(["B1"]));
    expect(lines).toHaveLength(0);
  });

  it("ignores resolutions that are not engagements", () => {
    const lines = fireLinesFor(
      { state: board(), events: [shot({ kind: "morale", targetIds: [] })] },
      ALL,
    );
    expect(lines).toHaveLength(0);
  });

  it("has nothing to draw for a step with no rolls", () => {
    expect(fireLinesFor({ state: board(), events: undefined }, ALL)).toEqual([]);
  });
});

describe("showing the dice", () => {
  it("names every modifier, because a total alone teaches nothing", () => {
    const text = describeRoll(shot());
    expect(text).toContain("4+3 = 7");
    expect(text).toContain("range -2");
    expect(text).toContain("total 5");
    expect(text).toContain("suppressed");
    // A modifier that did not apply is noise, not information.
    expect(text).not.toContain("targetInCover");
  });

  it("shows the dice as dice", () => {
    expect(pipsFor([1, 6])).toBe("\u2680 \u2685");
  });

  it("copes with a resolution that had no roll", () => {
    const text = describeRoll(shot({ roll: undefined, modifiers: [], total: 3 }));
    expect(text).toContain("total 3");
  });
});

describe("who may be shown what", () => {
  it("includes wrecks, which is why the killing shot had no line", () => {
    // The counter layer draws living elements, and the fire lines were filtered
    // through the same set — so the shot that destroyed something was the one
    // shot that could never be drawn.
    const wrecked = board();
    wrecked.forceElements.R1 = { ...wrecked.forceElements.R1, combatStrength: 0 };

    const ids = visibleIdsFor(wrecked, "both");
    expect(ids.has("R1")).toBe(true);

    const killing = shot({ effects: [{ kind: "combatStrength", feId: "R1", delta: -8 }] });
    const lines = fireLinesFor({ state: wrecked, events: [killing] }, ids);
    expect(lines).toHaveLength(1);
    expect(lines[0].outcome).toBe("destroyed");
    expect(lines[0].note).toBe("destroyed");
  });

  it("shows a side its own elements and whatever it has sighted", () => {
    const state = board();
    state.sighting = { blue: { R1: "full" }, red: {} };

    const blue = visibleIdsFor(state, "blue");
    expect(blue.has("B1")).toBe(true);
    expect(blue.has("R1")).toBe(true);
    expect(blue.has("R2")).toBe(false);

    expect(visibleIdsFor(state, "red").has("B1")).toBe(false);
  });

  it("lists the wrecks a viewpoint can see, and no others", () => {
    const state = board();
    state.forceElements.R1 = { ...state.forceElements.R1, combatStrength: 0 };
    state.forceElements.R2 = { ...state.forceElements.R2, combatStrength: 0 };

    expect(
      wrecksIn(state, visibleIdsFor(state, "both"))
        .map((wreck) => wreck.id)
        .sort(),
    ).toEqual(["R1", "R2"]);

    // Blue has only sighted one of them.
    state.sighting = { blue: { R1: "full" }, red: {} };
    expect(wrecksIn(state, visibleIdsFor(state, "blue")).map((wreck) => wreck.id)).toEqual([
      "R1",
    ]);
  });
});

describe("the timeline through the fog", () => {
  const state = board();

  function step(label: string, side?: "blue" | "red", actorId?: string) {
    return {
      turn: 2,
      phase: "arcAction" as const,
      label,
      state,
      side,
      actorId,
      events: [shot({ actorIds: [actorId ?? "B1"], targetIds: [] })],
    };
  }

  it("shows phase steps to everyone: the clock is not a secret", () => {
    const steps = [step("Turn 2 begins"), step("Sighting"), step("Clean-up")];
    expect(stepsVisibleTo(steps, "blue")).toHaveLength(3);
  });

  it("hides an enemy action taken out of sight", () => {
    // THE LEAK THIS FIXES: the map layers were careful about fog and the step
    // LIST was not, so "blue eyes only" listed every red action with red's
    // dice attached.
    const steps = [step("R1 fires on B1", "red", "R1"), step("B1 holds", "blue", "B1")];
    const blue = stepsVisibleTo(steps, "blue");
    expect(blue.map((entry) => entry.label)).toEqual(["B1 holds"]);
    // The umpire still sees both.
    expect(stepsVisibleTo(steps, "both")).toHaveLength(2);
  });

  it("shows a sighted enemy's action, but not its plan", () => {
    // You can watch a troop move. You cannot see where it means to be in four
    // turns, so the march's detail is cut at the bracket.
    const sighted = { ...board(), sighting: { blue: { R1: "full" as const }, red: {} } };
    const steps = [
      {
        turn: 2,
        phase: "arcAction" as const,
        label: "R1 continues to the objective (2,250 m, 4,100 m to go)",
        state: sighted,
        side: "red" as const,
        actorId: "R1",
      },
    ];
    expect(stepsVisibleTo(steps, "blue")[0].label).toBe("R1 continues to the objective");
    expect(stepsVisibleTo(steps, "both")[0].label).toContain("4,100 m to go");
  });

  it("keeps the real index, so selecting a step still addresses it", () => {
    const steps = [step("R1 fires", "red", "R1"), step("B1 holds", "blue", "B1")];
    expect(stepsVisibleTo(steps, "blue")[0].index).toBe(1);
  });

  it("shows the arithmetic of engagements you were in, and no others", () => {
    const mine = { state, events: [shot({ actorIds: ["B1"], targetIds: ["R1"] })] };
    const theirs = { state, events: [shot({ actorIds: ["R1"], targetIds: ["R2"] })] };
    expect(eventsVisibleTo(mine, "blue")).toHaveLength(1);
    expect(eventsVisibleTo(theirs, "blue")).toHaveLength(0);
    expect(eventsVisibleTo(theirs, "both")).toHaveLength(1);
  });
});
