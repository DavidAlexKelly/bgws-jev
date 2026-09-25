/**
 * A move stops when somebody sees somebody.
 *
 * The bug these tests pin down: a move used to be atomic, so two troops whose
 * routes crossed swapped positions inside a single turn — each arriving behind
 * the other, noticing only at the next turn's sighting sweep, by which time
 * they were past and facing the wrong way.
 */

import { describe, expect, it } from "vitest";

import { distanceM, metresPerDegreeLon, type LatLng } from "../lib/board";
import { flatTerrain, type TerrainSampler } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import type { DiceRoll, Rng } from "./dice";
import { EventLog } from "./events";
import { HOUSE_V1, withModules } from "./ruleset";
import { resolveAction, type PhaseConfig } from "./turnLoop";
import { walkUntilContact } from "./contact";

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
const METRES_PER_DEGREE_LAT = 111_320;

function at(east: number, north: number): LatLng {
  return {
    lat: ORIGIN.lat + north / METRES_PER_DEGREE_LAT,
    lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat),
  };
}

/** A generator that always rolls the same face: sighting becomes a certainty. */
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

function fe(id: string, side: Side, position: LatLng): ForceElement {
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
  };
}

function board(elements: ForceElement[], sighting?: GameState["sighting"]): GameState {
  return {
    gameId: "contact",
    scenarioId: "contact",
    turn: 2,
    phase: "arcAction",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((one) => [one.id, one])),
    sighting: sighting ?? { blue: {}, red: {} },
    rng: { seed: "contact", cursor: 0 },
  };
}

function walkOptions(terrain: TerrainSampler = flatTerrain(), rng: Rng = fixedRng(6)) {
  return {
    terrain,
    ruleset: HOUSE_V1,
    rng,
    turn: 2,
    phase: "arcAction" as const,
  };
}

describe("a move halts on contact", () => {
  it("stops short when it walks into something it had not seen", () => {
    // Red sits 1 km along blue's axis of advance. Blue is ordered 3 km up it.
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(1000, 0));
    const walk = walkUntilContact(board([mover, enemy]), mover, at(3000, 0), walkOptions());

    expect(walk.halted).toBe(true);
    expect(walk.contacts).toEqual(["R1"]);
    // It stopped somewhere short of where it was going.
    expect(walk.distanceM).toBeLessThan(2900);
    expect(distanceM(walk.end, at(3000, 0))).toBeGreaterThan(100);
  });

  it("reports the contact as a sighting effect, so the board agrees", () => {
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(600, 0));
    const walk = walkUntilContact(board([mover, enemy]), mover, at(2000, 0), walkOptions());

    expect(walk.sighted).toHaveLength(1);
    expect(walk.sighted[0]).toMatchObject({ kind: "sighting", viewer: "blue", feId: "R1" });
    expect(walk.events.length).toBeGreaterThan(0);
  });

  it("does NOT stop for an enemy it had already sighted", () => {
    // It saw them and its commander chose to move anyway. A rule that stopped
    // it would make movement impossible in contact, which BGWS allows.
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(1000, 0));
    const walk = walkUntilContact(
      board([mover, enemy], { blue: { R1: "full" }, red: {} }),
      mover,
      at(3000, 0),
      walkOptions(),
    );

    expect(walk.halted).toBe(false);
    expect(walk.distanceM).toBeCloseTo(distanceM(at(0, 0), at(3000, 0)), -1);
  });

  it("does not stop for something it cannot see", () => {
    // Line of sight decides. Thick woods between the two, and the march goes on.
    const blind: TerrainSampler = { groundHeightM: () => 0, classify: () => "woodsThick" };
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(1000, 400));
    const walk = walkUntilContact(
      board([mover, enemy]),
      mover,
      at(3000, 0),
      walkOptions(blind),
    );
    expect(walk.halted).toBe(false);
  });

  it("does not stop for a wreck", () => {
    const mover = fe("B1", "blue", at(0, 0));
    const dead = { ...fe("R1", "red", at(800, 0)), combatStrength: 0 };
    const walk = walkUntilContact(board([mover, dead]), mover, at(3000, 0), walkOptions());
    expect(walk.halted).toBe(false);
  });

  it("gives the same halt every time, so a replay is exact", () => {
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(1200, 0));
    const state = board([mover, enemy]);
    const first = walkUntilContact(state, mover, at(3000, 0), walkOptions());
    const second = walkUntilContact(state, mover, at(3000, 0), walkOptions());
    expect(second.end).toEqual(first.end);
    expect(second.distanceM).toBeCloseTo(first.distanceM, 6);
  });

  it("tries each enemy once per move, not once per step", () => {
    // A long march past one troop should not roll thirty times against it: the
    // element either sees it as it comes into view or it does not.
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(400, 2500));
    const walk = walkUntilContact(
      board([mover, enemy]),
      mover,
      at(4000, 0),
      // A roll of 1s never sights anything, so every step would retry.
      walkOptions(flatTerrain(), fixedRng(1)),
    );
    expect(walk.halted).toBe(false);
    expect(walk.events).toHaveLength(1);
  });
});

describe("halting through the turn loop", () => {
  function config(ruleset = HOUSE_V1): PhaseConfig {
    return {
      ruleset,
      terrain: flatTerrain(),
      rng: fixedRng(6),
      log: new EventLog(),
      maxTurns: 15,
    };
  }

  const march = {
    goal: at(4000, 0),
    label: "the objective",
    waypoints: [at(4000, 0)],
    plannedOnTurn: 1,
    planner: "bearings" as const,
  };

  it("abandons the march, because the plan was made before the enemy arrived", () => {
    const state = board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(900, 0))]);
    const next = resolveAction(
      state,
      {
        id: "B1:march",
        kind: "move",
        actorId: "B1",
        destination: at(2500, 0),
        route: march,
        summary: "B1 continues to the objective",
      },
      config(),
      2,
    );

    expect(next.forceElements.B1.route).toBeUndefined();
    // And it is not where it was ordered to be.
    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeGreaterThan(100);
  });

  it("keeps the march when nothing was seen", () => {
    const state = board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(9000, 9000))]);
    const next = resolveAction(
      state,
      {
        id: "B1:march",
        kind: "move",
        actorId: "B1",
        destination: at(2000, 0),
        route: march,
        summary: "B1 continues to the objective",
      },
      config(),
      2,
    );
    expect(next.forceElements.B1.route).toBeDefined();
  });

  it("drives straight past with the module off, which is the old behaviour", () => {
    // Kept measurable rather than deleted: this is the arm the sweep prices
    // the rule against, and it is exactly what the map used to show.
    const state = board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(900, 0))]);
    const next = resolveAction(
      state,
      {
        id: "B1:move",
        kind: "move",
        actorId: "B1",
        destination: at(2500, 0),
        summary: "B1 advances",
      },
      config(withModules(HOUSE_V1, { contactHalt: false })),
      2,
    );
    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeLessThan(5);
  });
});

describe("7.1.3's choice: press on, or go to ground", () => {
  it("presses on through contact when told to, and records it", () => {
    // "After it is resolved the moving FE may continue its movement ... or it
    // may elect to stop moving at that point." The rulebook asks the moving
    // player; this engine pre-commits the answer on the option.
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(800, 0));
    const walk = walkUntilContact(board([mover, enemy]), mover, at(3000, 0), {
      ...walkOptions(),
      haltOnContact: false,
    });

    expect(walk.halted).toBe(false);
    expect(walk.contacts).toEqual(["R1"]);
    // It went the whole way, and it knows what it drove past.
    expect(walk.distanceM).toBeCloseTo(distanceM(at(0, 0), at(3000, 0)), -1);
    expect(walk.sighted).toHaveLength(1);
  });

  it("collects every contact made along the way, not just the first", () => {
    const mover = fe("B1", "blue", at(0, 0));
    const near = fe("R1", "red", at(600, 0));
    const far = fe("R2", "red", at(2000, 200));
    const walk = walkUntilContact(board([mover, near, far]), mover, at(3000, 0), {
      ...walkOptions(),
      haltOnContact: false,
    });
    expect(walk.contacts.sort()).toEqual(["R1", "R2"]);
  });

  it("halts by default, which is what a commander with no preference gets", () => {
    const mover = fe("B1", "blue", at(0, 0));
    const enemy = fe("R1", "red", at(800, 0));
    const walk = walkUntilContact(board([mover, enemy]), mover, at(3000, 0), walkOptions());
    expect(walk.halted).toBe(true);
  });
});

describe("pressing on through the turn loop", () => {
  function config(ruleset = HOUSE_V1): PhaseConfig {
    return {
      ruleset,
      terrain: flatTerrain(),
      rng: fixedRng(6),
      log: new EventLog(),
      maxTurns: 15,
    };
  }

  const march = {
    goal: at(4000, 0),
    label: "the objective",
    waypoints: [at(4000, 0)],
    plannedOnTurn: 1,
    planner: "bearings" as const,
  };

  it("keeps going, and keeps its march, when the option says press on", () => {
    const state = board([fe("B1", "blue", at(0, 0)), fe("R1", "red", at(900, 0))]);
    const next = resolveAction(
      state,
      {
        id: "B1:march:press",
        kind: "move",
        actorId: "B1",
        destination: at(2500, 0),
        route: march,
        onContact: "press",
        summary: "B1 continues to the objective — press on through contact",
      },
      config(),
      2,
    );

    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeLessThan(5);
    expect(next.forceElements.B1.route).toBeDefined();
    // It still SAW them: pressing on is not ignoring.
    expect(next.sighting.blue.R1).not.toBe("none");
    expect(next.sighting.blue.R1).toBeDefined();
  });

  it("takes the choice away from an element that is Disrupted or Broken", () => {
    // 7.1.3: it may continue "unless it has become Disrupted or Broken".
    const shaken = { ...fe("B1", "blue", at(0, 0)), morale: "disrupted" as const };
    const state = board([shaken, fe("R1", "red", at(900, 0))]);
    const next = resolveAction(
      state,
      {
        id: "B1:march:press",
        kind: "move",
        actorId: "B1",
        destination: at(2500, 0),
        route: march,
        onContact: "press",
        summary: "B1 presses on",
      },
      config(),
      2,
    );

    expect(distanceM(next.forceElements.B1.position, at(2500, 0))).toBeGreaterThan(100);
    expect(next.forceElements.B1.route).toBeUndefined();
  });
});
