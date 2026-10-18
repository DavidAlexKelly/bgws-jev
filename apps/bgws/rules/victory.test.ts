/**
 * Who won, and how well.
 *
 * The rule these tests hold in place: TAKING THE GROUND OUTRANKS HURTING THE
 * ENEMY, and hurting the enemy outranks having more left over. The engine used
 * to decide every unresolved game on the last of those three, which made every
 * outcome figure in every report a measure of who did more damage.
 */

import { describe, expect, it } from "vitest";

import type { ForceElement, GameState, Side } from "../lib/state";
import { HOUSE_V1 } from "./ruleset";
import { describeVerdict, judgeVictory } from "./victory";

const OBJECTIVE = { blue: { lat: 54.75, lng: 20.5 }, red: { lat: 54.65, lng: 20.5 } };

function fe(
  id: string,
  side: Side,
  overrides: Partial<ForceElement> = {},
): ForceElement {
  return {
    id,
    side,
    label: id,
    sidc: "SFGPUCA-------",
    moveType: "T",
    targetClass: "armoured_vehicle",
    capabilities: [],
    troopQuality: 4,
    combatStrength: 10,
    combatStrengthStart: 10,
    morale: "good",
    markers: [],
    concealed: false,
    isDummy: false,
    position: { lat: 54.7, lng: 20.5 },
    ...overrides,
  };
}

function board(elements: ForceElement[]): GameState {
  return {
    gameId: "v",
    scenarioId: "v",
    turn: 9,
    phase: "cleanup",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((one) => [one.id, one])),
    sighting: { blue: {}, red: {} },
    rng: { seed: "v", cursor: 0 },
    objectives: OBJECTIVE,
  };
}

describe("annihilation", () => {
  it("is decisive for whoever is left", () => {
    const verdict = judgeVictory(
      board([fe("B1", "blue"), fe("R1", "red", { combatStrength: 0 })]),
      HOUSE_V1,
    );
    expect(verdict.winner).toBe("blue");
    expect(verdict.level).toBe("decisive");
    expect(verdict.basis).toBe("annihilation");
  });
});

describe("the ground decides first", () => {
  it("is substantive when the objective is held against a force still in being", () => {
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { position: OBJECTIVE.blue }),
        fe("R1", "red", { position: { lat: 54.6, lng: 20.5 } }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.winner).toBe("blue");
    expect(verdict.level).toBe("substantive");
    expect(verdict.basis).toBe("objectiveHeld");
  });

  it("is decisive when the objective is held AND the enemy is broken", () => {
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { position: OBJECTIVE.blue }),
        // A third of its starting strength left: no longer a force in being.
        fe("R1", "red", { combatStrength: 2, position: { lat: 54.6, lng: 20.5 } }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.level).toBe("decisive");
    expect(verdict.basis).toBe("objectiveAndEnemyBroken");
  });

  it("does not count an objective that is being contested", () => {
    // HOLDING MEANS HOLDING IT AGAINST SOMEBODY. Otherwise the rule rewards
    // driving past the enemy onto an undefended point.
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { position: OBJECTIVE.blue }),
        fe("R1", "red", { position: OBJECTIVE.blue }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.standing.blue.holdsObjective).toBe(false);
    expect(verdict.basis).not.toBe("objectiveHeld");
  });

  it("lets both sides hold their own objectives, which is a real outcome", () => {
    // The objectives are in different places, and neither side has denied the
    // other anything. 3.1: "It is possible for both sides to achieve some form
    // of success."
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { position: OBJECTIVE.blue }),
        fe("R1", "red", { position: OBJECTIVE.red }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.winner).toBeNull();
    expect(verdict.level).toBe("substantive");
    expect(describeVerdict(verdict)).toContain("both sides hold");
  });
});

describe("attrition is the last clause, not the first", () => {
  it("gives a marginal win when the enemy is broken but nothing was taken", () => {
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { position: { lat: 54.7, lng: 20.5 } }),
        fe("R1", "red", { combatStrength: 1, position: { lat: 54.69, lng: 20.5 } }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.winner).toBe("blue");
    expect(verdict.level).toBe("marginal");
    expect(verdict.basis).toBe("enemyBroken");
  });

  it("gives a marginal win on a wide effectiveness gap", () => {
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { combatStrength: 10 }),
        fe("R1", "red", { combatStrength: 6 }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.winner).toBe("blue");
    expect(verdict.basis).toBe("attrition");
    expect(verdict.level).toBe("marginal");
  });

  it("calls a narrow gap what it is: no result", () => {
    // The old engine called this a blue win, and every report counted it.
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { combatStrength: 10 }),
        fe("R1", "red", { combatStrength: 9 }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.winner).toBeNull();
    expect(verdict.level).toBe("none");
    expect(verdict.basis).toBe("stalemate");
    expect(describeVerdict(verdict)).toContain("no result");
  });
});

describe("the verdict shows its work", () => {
  it("reports each side's standing, so a screen can explain itself", () => {
    const verdict = judgeVictory(
      board([
        fe("B1", "blue", { position: OBJECTIVE.blue, combatStrength: 5 }),
        fe("R1", "red", { position: { lat: 54.6, lng: 20.5 } }),
      ]),
      HOUSE_V1,
    );
    expect(verdict.standing.blue.effectiveness).toBeCloseTo(0.5, 5);
    expect(verdict.standing.blue.nearestToObjectiveM).toBeLessThan(
      HOUSE_V1.victory.holdWithinM,
    );
    expect(verdict.standing.red.combatEffective).toBe(true);
  });

  it("copes with a scenario that has no objectives at all", () => {
    const state = board([fe("B1", "blue"), fe("R1", "red", { combatStrength: 4 })]);
    delete state.objectives;
    const verdict = judgeVictory(state, HOUSE_V1);
    expect(verdict.standing.blue.holdsObjective).toBe(false);
    expect(verdict.winner).toBe("blue"); // decided on attrition, as it must be
  });
});
