/**
 * INDIRECT FIRE AND SMOKE (Core Rules 9.2.2).
 *
 * The part of the game with no source behind it at all. Direct fire has the
 * L6 profiles — ranges, penetration, combat strength indices, measured by
 * somebody. The catalogue contains no mortar, no artillery piece and no
 * attack helicopter: a query for any platform whose id or name contains
 * "mortar" returns zero rows. The IDF columns of the Fire Results Table are
 * on Player Aid 4, which is not in the box we have.
 *
 * So these tests assert the rulebook's PROSE, which is all there is: what a
 * mortar may shoot at without seeing it, what smoke does to whom, and that a
 * mission can hurt the side that called it.
 */

import { describe, expect, it } from "vitest";

import { flatTerrain } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import { clearAllMarkers } from "./apply";
import { createRng } from "./dice";
import { EventLog, type ResolutionEvent } from "./events";
import { HOUSE_V1, withModules } from "./ruleset";
import {
  indirectFireOptionsFor,
  inSmoke,
  optionsFor,
  resolveAction,
  smokeOnLine,
  type PhaseConfig,
} from "./turnLoop";

const IDF = withModules(HOUSE_V1, { indirectFire: true });

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

const mortar = (overrides: Partial<ForceElement> = {}) =>
  fe({
    id: "M1",
    side: "blue",
    moveType: "W",
    targetClass: "soft_skin",
    position: { lat: 54.705, lng: 20.51 },
    capabilities: [
      { kind: "idf", maxRangeM: 3000, shortRangeM: 1500 },
      { kind: "smoke", maxRangeM: 3000, shortRangeM: 1500 },
    ],
    ...overrides,
  });

function board(elements: ForceElement[], sighted: "full" | "partial" = "full"): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  for (const element of elements) {
    const viewer: Side = element.side === "blue" ? "red" : "blue";
    sighting[viewer][element.id] = sighted;
  }
  return {
    gameId: "idf",
    scenarioId: "idf",
    turn: 1,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((f) => [f.id, f])),
    sighting,
    rng: { seed: "idf", cursor: 0 },
  };
}

function phaseConfig(ruleset = IDF, seed = "idf"): PhaseConfig {
  return {
    ruleset,
    terrain: flatTerrain(),
    rng: createRng(seed),
    log: new EventLog(),
    maxTurns: 40,
  };
}

/** A mortar, a friendly observer up front, and an enemy it cannot see itself. */
function battery(): GameState {
  return board([
    mortar(),
    fe({ id: "B2", side: "blue", position: { lat: 54.712, lng: 20.51 } }),
    fe({ id: "R1", side: "red", position: { lat: 54.715, lng: 20.51 } }),
  ]);
}

// ── 9.2.2.1 A mortar does not need to see ──────────────────────────────────

describe("what a mortar may engage (9.2.2.1)", () => {
  it("fires at a target a FRIEND can see", () => {
    // "An Activated Mortar FE can IDF at a target in Range without LoS,
    // provided a friendly FE has LoS to the target." The whole point of the
    // weapon, and the reason indirect fire cannot reuse the direct fire
    // option builder.
    const state = battery();
    const options = indirectFireOptionsFor(state, state.forceElements.M1, phaseConfig());
    expect(options.some((option) => option.id === "M1:idf:R1")).toBe(true);
  });

  it("fires at a target that is only PARTIALLY sighted", () => {
    // "Can be used against Concealed enemy FEs that have been 'Partially
    // Sighted'". Direct fire cannot do this at all.
    const state = board(
      [
        mortar(),
        fe({ id: "B2", side: "blue", position: { lat: 54.712, lng: 20.51 } }),
        fe({ id: "R1", side: "red", position: { lat: 54.715, lng: 20.51 } }),
      ],
      "partial",
    );
    const options = indirectFireOptionsFor(state, state.forceElements.M1, phaseConfig());
    expect(options.some((option) => option.id === "M1:idf:R1")).toBe(true);
  });

  it("does not fire at something NOBODY on its side can see", () => {
    const state = battery();
    const blind = { ...state, sighting: { blue: {}, red: state.sighting.red } };
    const options = indirectFireOptionsFor(blind, blind.forceElements.M1, phaseConfig());
    expect(options).toEqual([]);
  });

  it("does not fire beyond its own range, however well observed", () => {
    const state = board([
      mortar({ capabilities: [{ kind: "idf", maxRangeM: 500, shortRangeM: 250 }] }),
      fe({ id: "B2", side: "blue", position: { lat: 54.712, lng: 20.51 } }),
      fe({ id: "R1", side: "red", position: { lat: 54.715, lng: 20.51 } }),
    ]);
    expect(indirectFireOptionsFor(state, state.forceElements.M1, phaseConfig())).toEqual([]);
  });

  it("offers nothing when the module is off", () => {
    const state = battery();
    expect(indirectFireOptionsFor(state, state.forceElements.M1, phaseConfig(HOUSE_V1))).toEqual(
      [],
    );
  });

  it("offers nothing to an element with no tube", () => {
    const state = battery();
    expect(indirectFireOptionsFor(state, state.forceElements.B2, phaseConfig())).toEqual([]);
  });

  it("is offered alongside everything else a mortar could do", () => {
    const state = battery();
    const all = optionsFor(state, state.forceElements.M1, phaseConfig());
    expect(all.some((option) => option.indirect)).toBe(true);
  });
});

// ── 9.2.2 Area effect ──────────────────────────────────────────────────────

describe("a mission lands on an area, not a man (9.2.2)", () => {
  it("can shake the neighbours of the element it hit — INCLUDING FRIENDS", () => {
    // "other FEs (Friendly or Enemy) within 250m of it may take a level of
    // Morale Status loss." The first rule in the game that can hurt your own
    // side, and the reason to think before dropping fire onto a melee.
    let hurtSomeoneFriendly = false;

    for (let seed = 0; seed < 30 && !hurtSomeoneFriendly; seed += 1) {
      const state = board([
        mortar(),
        // A blue element standing right next to the red target.
        fe({ id: "B2", side: "blue", position: { lat: 54.7151, lng: 20.51 } }),
        fe({ id: "R1", side: "red", position: { lat: 54.715, lng: 20.51 } }),
      ]);
      const config = phaseConfig(IDF, `splash${seed}`);
      const option = indirectFireOptionsFor(state, state.forceElements.M1, config).find(
        (candidate) => candidate.id === "M1:idf:R1",
      )!;

      const next = resolveAction(state, option, config, 1);
      if (next.forceElements.B2.morale !== "good") hurtSomeoneFriendly = true;
    }

    expect(hurtSomeoneFriendly).toBe(true);
  });

  it("leaves elements outside the radius alone", () => {
    // 1,100 m away: well outside 250 m, and it should never be touched.
    for (let seed = 0; seed < 20; seed += 1) {
      const state = board([
        mortar(),
        fe({ id: "B2", side: "blue", position: { lat: 54.725, lng: 20.51 } }),
        fe({ id: "R1", side: "red", position: { lat: 54.715, lng: 20.51 } }),
      ]);
      const config = phaseConfig(IDF, `far${seed}`);
      const option = indirectFireOptionsFor(state, state.forceElements.M1, config).find(
        (candidate) => candidate.id === "M1:idf:R1",
      )!;
      expect(resolveAction(state, option, config, 1).forceElements.B2.morale).toBe("good");
    }
  });
});

// ── 9.2.2.4 Smoke ──────────────────────────────────────────────────────────

describe("smoke (9.2.2.4)", () => {
  const laySmoke = () => {
    const state = battery();
    const config = phaseConfig();
    const option = indirectFireOptionsFor(state, state.forceElements.M1, config).find(
      (candidate) => candidate.smoke,
    )!;
    return { next: resolveAction(state, option, config, 1), config };
  };

  it("is offered by a tube that carries it", () => {
    const state = battery();
    const options = indirectFireOptionsFor(state, state.forceElements.M1, phaseConfig());
    expect(options.some((option) => option.smoke)).toBe(true);
  });

  it("needs no roll, and kills nobody", () => {
    // "No roll is required."
    const { next, config } = laySmoke();
    const events = config.log
      .all()
      .filter((e): e is ResolutionEvent => e.type === "resolution");
    const [smokeEvent] = events;
    expect(smokeEvent.table).toBe("idf:smoke");
    expect(smokeEvent.roll?.dice ?? []).toEqual([]);
    expect(next.forceElements.R1.combatStrength).toBe(8);
  });

  it("puts a cloud on the map that covers the ground around the target", () => {
    const { next } = laySmoke();
    expect(next.smoke ?? []).toHaveLength(1);
    expect(inSmoke(next, next.forceElements.R1.position, IDF)).toBe(true);
  });

  it("blocks a line that passes through it, not only one that ends in it", () => {
    // "any FE firing from, through or into Smoke suffers a -2 DRM" — all
    // three cases, which is why the line is sampled and not just its ends.
    const { next } = laySmoke();
    const through = smokeOnLine(
      next,
      { lat: 54.705, lng: 20.51 },
      { lat: 54.725, lng: 20.51 },
      IDF,
    );
    expect(through).toBe(true);
  });

  it("does not affect a line nowhere near it", () => {
    const { next } = laySmoke();
    const clear = smokeOnLine(next, { lat: 54.7, lng: 20.6 }, { lat: 54.71, lng: 20.6 }, IDF);
    expect(clear).toBe(false);
  });

  it("penalises a shot fired through it", () => {
    const { next, config } = laySmoke();
    // B2 shoots R1; the cloud sits on R1, so the round arrives through smoke.
    const shot = optionsFor(next, next.forceElements.B2, config).find(
      (option) => option.kind === "fire" && option.targetId === "R1",
    )!;
    resolveAction(next, shot, config, 1);

    const shots = config.log
      .all()
      .filter((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "directFire");
    const fire = shots[shots.length - 1];
    expect(fire.modifiers.some((m: { source: string }) => m.source === "smoke")).toBe(true);
  });

  it("is gone by the next turn", () => {
    // 8.0 step 1: "Remove all MOVED, FIRED and SMOKE markers from the map."
    // A cloud lasts the turn it was fired and no longer.
    const { next } = laySmoke();
    expect(clearAllMarkers(next, true).smoke).toEqual([]);
  });

  it("does nothing at all when the module is off", () => {
    const { next } = laySmoke();
    expect(inSmoke(next, next.forceElements.R1.position, HOUSE_V1)).toBe(false);
    expect(smokeOnLine(next, next.forceElements.M1.position, next.forceElements.R1.position, HOUSE_V1)).toBe(
      false,
    );
  });
});
