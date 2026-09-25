/**
 * RALLY (5.2) — the only way back UP the morale ladder.
 *
 * Before this existed, morale was a one-way ratchet for the commonest damaged
 * state on the board. Fire's "suppress" result costs no Combat Strength, and
 * clean-up's morale check only tests elements that have LOST strength, so an
 * element that was shaken but intact had no check to take and no way back. It
 * stayed suppressed for the rest of the game.
 *
 * Every test here is about a promise the rulebook makes and the engine did
 * not keep.
 */

import { describe, expect, it } from "vitest";

import { flatTerrain, type TerrainSampler } from "../lib/lineOfSight";
import type { DiceRoll, Rng } from "./dice";
import type { ForceElement, GameState, Morale, Side } from "../lib/state";
import { EventLog } from "./events";
import { HOUSE_V1, withModules } from "./ruleset";
import { runRally, type PhaseConfig } from "./turnLoop";

/** A generator that always rolls the same face, so a rule can be read off. */
function fixedRng(face: number): Rng {
  const roll = (dice: number): DiceRoll => ({
    dice: Array.from({ length: dice }, () => face),
    total: face * dice,
    cursor: 0,
  });
  return {
    seed: `fixed-${face}`,
    cursor: 0,
    d6: () => roll(1),
    d66: () => roll(2),
    int: () => 0,
    pick: <T,>(items: readonly T[]): T => items[0],
  };
}

function fe(overrides: Partial<ForceElement> & { id: string; side: Side }): ForceElement {
  return {
    label: overrides.id,
    sidc: "SFGPUCA-------",
    moveType: "T",
    targetClass: "armoured_vehicle",
    capabilities: [{ kind: "atk", maxRangeM: 3000, shortRangeM: 1500 }],
    troopQuality: 3,
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
  return {
    gameId: "rally",
    scenarioId: "rally",
    turn: 3,
    phase: "command",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((one) => [one.id, one])),
    sighting: { blue: {}, red: {} },
    rng: { seed: "rally", cursor: 0 },
  };
}

function config(rng: Rng, terrain: TerrainSampler = flatTerrain()): PhaseConfig {
  return { ruleset: HOUSE_V1, terrain, rng, log: new EventLog(), maxTurns: 15 };
}

/** Far enough that no line of sight reaches (the cap is 3 km). */
const FAR = { lat: 54.78, lng: 20.62 };
const NEAR_ENEMY = { lat: 54.715, lng: 20.51 };

function moraleOf(state: GameState, id: string): Morale {
  return state.forceElements[id].morale;
}

describe("a shaken element can recover at all", () => {
  it("rallies an element that was suppressed without being damaged", () => {
    // THE BUG. Undamaged, so clean-up's morale check skips it forever; and
    // before 5.2 was implemented nothing else could touch its morale.
    const state = board([
      fe({ id: "B1", side: "blue", morale: "suppressed2", position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const next = runRally(state, config(fixedRng(5)), 3);
    expect(moraleOf(next, "B1")).toBe("suppressed1");
  });

  it("leaves an element at good morale alone", () => {
    // "each FE with a Morale Status marker" — no marker, nothing to recover,
    // and no roll to spend.
    const log = new EventLog();
    const state = board([fe({ id: "B1", side: "blue" })]);
    const cfg = { ...config(fixedRng(6)), log };
    const next = runRally(state, cfg, 3);
    expect(moraleOf(next, "B1")).toBe("good");
    expect(log.all()).toHaveLength(0);
  });

  it("never makes things worse, however badly it rolls", () => {
    // A Morale Check can break you. A Rally is an ATTEMPT to recover, and
    // failing recovers nothing — which is what stops "shaken" from being a
    // death spiral with no exit.
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const next = runRally(state, config(fixedRng(1)), 3);
    expect(moraleOf(next, "B1")).toBe("disrupted");
  });

  it("brings a broken element back, so broken is not absorbing", () => {
    const state = board([
      fe({ id: "B1", side: "blue", morale: "broken", position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const next = runRally(state, config(fixedRng(6)), 3);
    expect(moraleOf(next, "B1")).toBe("disrupted");
  });
});

describe("breaking contact is worth doing", () => {
  it("recovers a level automatically when no enemy can see it", () => {
    // 5.2's worked example: out of LoS of any enemy FE is an automatic level,
    // free of the dice. This is the rule that makes pulling back a move.
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: FAR }),
    ]);
    // Rolling a 1 with no quality DRM fails the 4+, so anything gained here
    // is the automatic level and nothing else.
    const next = runRally(state, config(fixedRng(1)), 3);
    expect(moraleOf(next, "B1")).toBe("suppressed2");
  });

  it("recovers two levels when out of contact AND the roll passes", () => {
    // "This means an FE can recover two levels of Morale Status with one
    // Rally" — 5.2's own play note.
    const state = board([
      fe({ id: "B1", side: "blue", morale: "broken", position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: FAR }),
    ]);
    const next = runRally(state, config(fixedRng(6)), 3);
    expect(moraleOf(next, "B1")).toBe("suppressed2");
  });

  it("gives nothing automatic while an enemy still has line of sight", () => {
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const next = runRally(state, config(fixedRng(1)), 3);
    expect(moraleOf(next, "B1")).toBe("disrupted");
  });

  it("counts only living enemies as watchers", () => {
    // A destroyed element is not observing anything, and leaving it in the
    // count would make a dead enemy pin a live one indefinitely.
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 }, combatStrength: 0 }),
    ]);
    const next = runRally(state, config(fixedRng(1)), 3);
    expect(moraleOf(next, "B1")).toBe("suppressed2");
  });
});

describe("an HQ steadies what it is with", () => {
  it("recovers a level automatically when co-located with an unsuppressed HQ", () => {
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: NEAR_ENEMY }),
      fe({ id: "HQ", side: "blue", commandRating: 3, position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const next = runRally(state, config(fixedRng(1)), 3);
    expect(moraleOf(next, "B1")).toBe("suppressed2");
  });

  it("gets nothing from an HQ that is itself suppressed", () => {
    // "an Un-Suppressed HQ". A shaken commander steadies nobody.
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: NEAR_ENEMY }),
      fe({ id: "HQ", side: "blue", commandRating: 3, morale: "suppressed1", position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const next = runRally(state, config(fixedRng(1)), 3);
    expect(moraleOf(next, "B1")).toBe("disrupted");
  });
});

describe("troop quality shows up in the roll", () => {
  it("lets good troops rally on a roll poor troops fail", () => {
    // The roll is a single D6 against 4+, so a +/-1 is worth a sixth of the
    // die — which is why quality is compressed rather than added raw.
    const veteran = board([
      fe({ id: "B1", side: "blue", morale: "suppressed2", troopQuality: 5, position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);
    const conscript = board([
      fe({ id: "B1", side: "blue", morale: "suppressed2", troopQuality: 2, position: NEAR_ENEMY }),
      fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.51 } }),
    ]);

    expect(moraleOf(runRally(veteran, config(fixedRng(3)), 3), "B1")).toBe("suppressed1");
    expect(moraleOf(runRally(conscript, config(fixedRng(4)), 3), "B1")).toBe("suppressed2");
  });
});

describe("when rally does not happen", () => {
  it("is skipped on turn 1, as the rulebook says", () => {
    const log = new EventLog();
    const state = board([fe({ id: "B1", side: "blue", morale: "suppressed1" })]);
    const next = runRally(state, { ...config(fixedRng(6)), log }, 1);
    expect(moraleOf(next, "B1")).toBe("suppressed1");
    expect(log.all()).toHaveLength(0);
  });

  it("is skipped with the module off, so the sweep can price it", () => {
    const state = board([fe({ id: "B1", side: "blue", morale: "suppressed1" })]);
    const off = { ...config(fixedRng(6)), ruleset: withModules(HOUSE_V1, { rally: false }) };
    expect(moraleOf(runRally(state, off, 3), "B1")).toBe("suppressed1");
  });

  it("does not rally the dead", () => {
    const state = board([
      fe({ id: "B1", side: "blue", morale: "broken", combatStrength: 0 }),
    ]);
    expect(moraleOf(runRally(state, config(fixedRng(6)), 3), "B1")).toBe("broken");
  });
});

describe("a rally is explainable", () => {
  it("logs what recovered the element, and by how much", () => {
    const log = new EventLog();
    const state = board([
      fe({ id: "B1", side: "blue", morale: "disrupted", position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: FAR }),
    ]);
    runRally(state, { ...config(fixedRng(6)), log }, 3);

    const [event] = log.all();
    expect(event.type).toBe("resolution");
    if (event.type !== "resolution") return;
    expect(event.kind).toBe("rally");
    expect(event.phase).toBe("command");
    expect(event.modifiers.map((m) => m.source)).toContain("outOfContact");
    expect(event.narrative).toContain("out of contact");
  });
});
