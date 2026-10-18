/**
 * WHAT CAN SHOOT WHAT (Core Rules 9.2.1, 2.1.8).
 *
 * "Remember:
 *    Apers: Apers Capabilities may only be used against Foot FEs and
 *           soft-skinned Wheeled vehicles.
 *    Atk:   Atk Capabilities may be used against any vehicle FE, but not
 *           Foot FEs."
 *
 * ⚠ `targetClass` WAS READ BY NO RULE, AND A COMMENT SAID IT WAS.
 *
 * It has been on every Force Element since the start, is parsed out of the L6
 * profiles and displayed in the Asset Explorer. Nothing consulted it — while
 * PenetrationRule's doc asserted that "`targetClass` gated WHETHER you could
 * engage", so the one place a reader would check reported the rule as done.
 *
 * It cost nothing while every force list was pure armour, which is how it
 * survived. It decides a great deal now that infantry exist.
 */

import { describe, expect, it } from "vitest";

import { flatTerrain } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import { createRng } from "./dice";
import { EventLog } from "./events";
import { COMBINED_ARMS_V1, PLATFORM_SNAPSHOT } from "./forceList";
import { HOUSE_V1 } from "./ruleset";
import { capabilityCanEngage, optionsFor, type PhaseConfig } from "./turnLoop";

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
    gameId: "tc",
    scenarioId: "tc",
    turn: 1,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((f) => [f.id, f])),
    sighting,
    rng: { seed: "tc", cursor: 0 },
  };
}

const config: PhaseConfig = {
  ruleset: HOUSE_V1,
  terrain: flatTerrain(),
  rng: createRng("tc"),
  log: new EventLog(),
  maxTurns: 40,
};

describe("the rulebook's two sentences", () => {
  it("apers may engage foot and soft skin, and nothing else", () => {
    expect(capabilityCanEngage("apers", "foot")).toBe(true);
    expect(capabilityCanEngage("apers", "soft_skin")).toBe(true);
    expect(capabilityCanEngage("apers", "armoured_vehicle")).toBe(false);
  });

  it("atk may engage any vehicle, but never foot", () => {
    expect(capabilityCanEngage("atk", "armoured_vehicle")).toBe(true);
    expect(capabilityCanEngage("atk", "soft_skin")).toBe(true);
    expect(capabilityCanEngage("atk", "foot")).toBe(false);
  });

  it("treats anti-tank missiles as atk", () => {
    expect(capabilityCanEngage("atm", "armoured_vehicle")).toBe(true);
    expect(capabilityCanEngage("atm", "foot")).toBe(false);
  });

  it("will not let smoke shoot anybody", () => {
    // Not a weapon. An element whose only capability is smoke cannot engage,
    // which is correct rather than a bug.
    expect(capabilityCanEngage("smoke", "foot")).toBe(false);
    expect(capabilityCanEngage("smoke", "armoured_vehicle")).toBe(false);
  });

  it("fails OPEN for capabilities it does not model", () => {
    // Same policy as unknown penetration, and for the same reason: a unit
    // whose only listed capability is `aa` would otherwise be silently unable
    // to fire at anything for its whole life.
    expect(capabilityCanEngage("aa", "armoured_vehicle")).toBe(true);
    expect(capabilityCanEngage("air_delivered", "foot")).toBe(true);
  });
});

describe("what it means on the board", () => {
  /** A tank with a main gun and a coax, facing something 400 m away. */
  const tank = () =>
    fe({
      id: "B1",
      side: "blue",
      capabilities: [
        { kind: "atk", maxRangeM: 3000, shortRangeM: 3000 },
        { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
      ],
    });

  const infantry = (id: string, side: Side) =>
    fe({
      id,
      side,
      moveType: "F",
      targetClass: "foot",
      combatStrength: 3,
      combatStrengthStart: 3,
      position: { lat: 54.7136, lng: 20.51 },
      capabilities: [
        { kind: "atm", maxRangeM: 800, shortRangeM: 400, penetrationMm: 500 },
        { kind: "apers", maxRangeM: 600, shortRangeM: 300 },
      ],
    });

  it("a tank may still engage infantry — with the coax, not the main gun", () => {
    // The rule does not stop a tank shooting infantry. It stops it doing so
    // with a weapon designed to defeat armour.
    const state = board([tank(), infantry("R1", "red")]);
    const fire = optionsFor(state, state.forceElements.B1, config).filter(
      (option) => option.kind === "fire",
    );
    expect(fire.length).toBeGreaterThan(0);
  });

  it("a tank with ONLY a main gun cannot engage infantry at all", () => {
    const state = board([
      fe({ id: "B1", side: "blue", capabilities: [{ kind: "atk", maxRangeM: 3000, shortRangeM: 3000 }] }),
      infantry("R1", "red"),
    ]);
    const fire = optionsFor(state, state.forceElements.B1, config).filter(
      (option) => option.kind === "fire",
    );
    expect(fire).toEqual([]);
  });

  it("infantry may engage a tank — with the missile, not the rifles", () => {
    const state = board([infantry("B1", "blue"), fe({ id: "R1", side: "red" })]);
    const fire = optionsFor(state, state.forceElements.B1, config).filter(
      (option) => option.kind === "fire",
    );
    expect(fire.length).toBeGreaterThan(0);
  });

  it("infantry out of missile range cannot touch a tank with rifles", () => {
    // Rifles reach 600 m and cannot hurt armour; the missile reaches 800 m.
    // At 1,200 m the section has nothing, which is the point of the rule.
    const state = board([
      infantry("B1", "blue"),
      fe({ id: "R1", side: "red", position: { lat: 54.7244, lng: 20.51 } }),
    ]);
    const fire = optionsFor(state, state.forceElements.B1, config).filter(
      (option) => option.kind === "fire",
    );
    expect(fire).toEqual([]);
  });
});

describe("the combined arms force list", () => {
  it("fields infantry on both sides, which no other list does", () => {
    const foot = COMBINED_ARMS_V1.elements.filter(
      (element) => PLATFORM_SNAPSHOT[element.platform].targetClass === "foot",
    );
    expect(foot.filter((element) => element.side === "blue").length).toBeGreaterThan(0);
    expect(foot.filter((element) => element.side === "red").length).toBeGreaterThan(0);
  });

  it("declares that its infantry figures are ours and not the source's", () => {
    // The source names 76 foot platforms and models none of them. Saying so
    // on the row is the difference between a known limitation and a quiet lie.
    expect(PLATFORM_SNAPSHOT["infantry/rifle_infantry"].valuesDeclared).toBeTruthy();
  });

  it("is symmetric, so it can be read as a control", () => {
    const strength = (side: Side) =>
      COMBINED_ARMS_V1.elements
        .filter((element) => element.side === side)
        .reduce((sum, element) => sum + element.platformCount, 0);
    expect(strength("blue")).toBe(strength("red"));
  });
});
