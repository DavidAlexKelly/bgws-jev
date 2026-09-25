/**
 * What happens AFTER an assault (Core Rules 9.3.6-9.3.10, and 8.0's clean-up).
 *
 * An assault used to be a roll with no aftermath. A broken defender stayed
 * exactly where it was, a repulsed attacker sat on the objective it had just
 * failed to take, and the MELEE marker — which 9.3.8 says in as many words is
 * NOT removed at the end of the turn — was wiped by clean-up every turn, so
 * close combat never outlived the roll that started it.
 */

import { describe, expect, it } from "vitest";

import { distanceM } from "../lib/board";
import { flatTerrain } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import { applyEffects, clearAllMarkers } from "./apply";
import { createRng } from "./dice";
import { EventLog } from "./events";
import { HOUSE_V1, withModules } from "./ruleset";
import { endStaleMelees, optionsFor, resolveAction, type PhaseConfig } from "./turnLoop";

const CLOSE = withModules(HOUSE_V1, { closeCombat: true });

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
    gameId: "cc",
    scenarioId: "cc",
    turn: 1,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((f) => [f.id, f])),
    sighting,
    rng: { seed: "cc", cursor: 0 },
  };
}

function phaseConfig(ruleset = CLOSE, seed = "cc"): PhaseConfig {
  return {
    ruleset,
    terrain: flatTerrain(),
    rng: createRng(seed),
    log: new EventLog(),
    maxTurns: 40,
  };
}

/** Two elements locked together, 100 m apart — inside the 250 m radius. */
function melee(): GameState {
  return board([
    fe({ id: "B1", side: "blue", markers: ["melee"], position: { lat: 54.71, lng: 20.51 } }),
    fe({ id: "R1", side: "red", markers: ["melee"], position: { lat: 54.7109, lng: 20.51 } }),
  ]);
}

const kinds = (options: ReturnType<typeof optionsFor>) =>
  [...new Set(options.map((option) => option.kind))].sort();

// ── 8.0 Clean-up ───────────────────────────────────────────────────────────

describe("clean-up keeps what 8.0 says to keep", () => {
  it("keeps the MELEE marker", () => {
    // "MELEE markers remain in place." The one marker that outlives its turn,
    // and wiping it meant close combat could not last longer than one roll.
    const state = applyEffects(board([fe({ id: "B1", side: "blue" })]), [
      { kind: "marker", feId: "B1", marker: "melee", added: true },
      { kind: "marker", feId: "B1", marker: "fired", added: true },
      { kind: "marker", feId: "B1", marker: "moved", added: true },
    ]);
    const cleaned = clearAllMarkers(state, true);
    expect(cleaned.forceElements.B1.markers).toEqual(["melee"]);
  });

  it("keeps a REORG placed this turn, and drops it the turn after", () => {
    // "Remove REORG markers if they were placed as a result of ... an Assault
    // last Turn (ones placed following an Assault this Turn remain in place)."
    // REORG has to survive one clean-up so that it restricts the element for
    // the "full subsequent Turn" 9.3.10 requires.
    const placed = applyEffects(board([fe({ id: "B1", side: "blue" })]), [
      { kind: "marker", feId: "B1", marker: "reorg", added: true },
      { kind: "marker", feId: "B1", marker: "reorgPlacedThisTurn", added: true },
    ]);

    const afterAssaultTurn = clearAllMarkers(placed, true);
    expect(afterAssaultTurn.forceElements.B1.markers).toEqual(["reorg"]);

    const afterTheTurnItSatOut = clearAllMarkers(afterAssaultTurn, true);
    expect(afterTheTurnItSatOut.forceElements.B1.markers).toEqual([]);
  });

  it("still wipes everything when the module is off", () => {
    // The old behaviour exactly, so the module can be measured against it.
    const state = applyEffects(board([fe({ id: "B1", side: "blue" })]), [
      { kind: "marker", feId: "B1", marker: "melee", added: true },
    ]);
    expect(clearAllMarkers(state, false).forceElements.B1.markers).toEqual([]);
  });
});

// ── 9.3.8 Melee ────────────────────────────────────────────────────────────

describe("an element locked in a melee (9.3.8)", () => {
  it("may only retreat, shoot what it is locked with, or press the assault", () => {
    // "The only Action they can take while they have a MELEE marker is on
    // subsequent Turns to Retreat, DirF against the enemy FE that are
    // currently in the Melee, or continue with the Assault."
    const state = melee();
    const options = optionsFor(state, state.forceElements.B1, phaseConfig());

    expect(kinds(options)).toEqual(["assault", "fire", "hold", "move"]);
    // The only move on offer is the retreat.
    const moves = options.filter((option) => option.kind === "move");
    expect(moves.length).toBe(1);
    expect(moves[0].retreat).toBe(true);
    // Everything it can shoot at is in the melee with it.
    for (const option of options.filter((o) => o.kind === "fire")) {
      expect(option.targetId).toBe("R1");
    }
  });

  it("is not restricted once there is nobody left to be locked with", () => {
    // 9.3.8's marker "remains in place ... until one side is Eliminated".
    // A stale marker would fix an element in place against an enemy that no
    // longer exists.
    const state = melee();
    const alone = applyEffects(state, [{ kind: "eliminated", feId: "R1" }]);
    const options = optionsFor(alone, alone.forceElements.B1, phaseConfig());
    expect(options.every((option) => option.retreat !== true)).toBe(true);
  });

  it("has the marker swept once the fight is over", () => {
    const state = melee();
    const alone = applyEffects(state, [{ kind: "eliminated", feId: "R1" }]);
    const swept = endStaleMelees(alone, phaseConfig());
    expect(swept.forceElements.B1.markers).not.toContain("melee");
  });

  it("keeps the marker while the enemy is still there", () => {
    const swept = endStaleMelees(melee(), phaseConfig());
    expect(swept.forceElements.B1.markers).toContain("melee");
    expect(swept.forceElements.R1.markers).toContain("melee");
  });

  it("is not restricted at all when the module is off", () => {
    // With closeCombat off a melee marker means what it used to: nothing much,
    // and cleared at the end of the turn anyway.
    const state = melee();
    const options = optionsFor(state, state.forceElements.B1, phaseConfig(HOUSE_V1));
    expect(options.some((option) => option.retreat === true)).toBe(false);
  });
});

// ── 9.3.6 Retreat ──────────────────────────────────────────────────────────

describe("breaking off (9.3.6)", () => {
  it("moves at least the rulebook's 500 m, away from the enemy", () => {
    const state = melee();
    const config = phaseConfig();
    const retreat = optionsFor(state, state.forceElements.B1, config).find(
      (option) => option.retreat,
    );
    expect(retreat).toBeDefined();

    const before = distanceM(state.forceElements.B1.position, state.forceElements.R1.position);
    const next = resolveAction(state, retreat!, config, 1);
    const after = distanceM(next.forceElements.B1.position, next.forceElements.R1.position);

    expect(after).toBeGreaterThan(before);
    expect(distanceM(state.forceElements.B1.position, next.forceElements.B1.position)).toBeCloseTo(
      CLOSE.assault.retreatMinM,
      -1,
    );
  });

  it("costs a step of morale and breaks the lock", () => {
    // "Any Retreating FE(s) drops one additional level of Morale Status."
    // Charged on a declared retreat as well as a forced one — otherwise
    // declaring would be strictly better than being made to, and nobody would
    // ever choose to fight on.
    const state = melee();
    const config = phaseConfig();
    const retreat = optionsFor(state, state.forceElements.B1, config).find(
      (option) => option.retreat,
    )!;

    const next = resolveAction(state, retreat, config, 1);
    expect(next.forceElements.B1.morale).toBe("suppressed1");
    expect(next.forceElements.B1.markers).not.toContain("melee");
  });
});

// ── 7.1.2 REORG ────────────────────────────────────────────────────────────

describe("reorganising after an assault (7.1.2, 9.3.10)", () => {
  const reorganising = () =>
    board([
      fe({ id: "B1", side: "blue", markers: ["reorg"], position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: { lat: 54.715, lng: 20.51 } }),
    ]);

  it("may fire and do nothing else", () => {
    // "it cannot take an Action that Turn except to DirF and Attempt
    // Sighting." Attempt Sighting is not an Action and is not offered as one.
    const state = reorganising();
    const options = optionsFor(state, state.forceElements.B1, phaseConfig());
    expect(kinds(options)).toEqual(["fire", "hold"]);
  });

  it("is unrestricted when the module is off", () => {
    const state = reorganising();
    const options = optionsFor(state, state.forceElements.B1, phaseConfig(HOUSE_V1));
    expect(kinds(options)).toContain("move");
  });
});
