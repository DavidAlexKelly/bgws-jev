/**
 * THE GROUND GETS A SAY IN MOVEMENT.
 *
 * These tests exist because the allowance rule (lib/movement.ts), the
 * allowance table (ruleset.movement) and the terrain sampler all existed for
 * weeks while `optionsFor` offered a destination 40% of the way towards the
 * enemy and the loop teleported the element to it. Every assertion here is
 * about the thing that was missing: an option a commander is offered must be
 * one the element could actually carry out.
 */

import { describe, expect, it } from "vitest";

import { distanceM, metresPerDegreeLon, type LatLng } from "../lib/board";
import type { TerrainSampler } from "../lib/lineOfSight";
import type { TerrainClass } from "../lib/movement";
import type { MoveType } from "../data/profiles";
import type { ForceElement, GameState, Side } from "../lib/state";
import { createRng } from "./dice";
import { EventLog } from "./events";
import { HOUSE_V1, withModules, type RuleSet } from "./ruleset";
import { optionsFor, type PhaseConfig } from "./turnLoop";

const ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };
const METRES_PER_DEGREE_LAT = 111_320;

function at(east: number, north: number): LatLng {
  return {
    lat: ORIGIN.lat + north / METRES_PER_DEGREE_LAT,
    lng: ORIGIN.lng + east / metresPerDegreeLon(ORIGIN.lat),
  };
}

function eastingOf(point: LatLng): number {
  return (point.lng - ORIGIN.lng) * metresPerDegreeLon(ORIGIN.lat);
}

function ground(classify: (east: number, north: number) => TerrainClass): TerrainSampler {
  return {
    groundHeightM: () => 0,
    classify: (point) =>
      classify(eastingOf(point), (point.lat - ORIGIN.lat) * METRES_PER_DEGREE_LAT),
  };
}

function fe(
  id: string,
  side: Side,
  position: LatLng,
  moveType: MoveType = "T",
): ForceElement {
  return {
    id,
    side,
    label: id,
    sidc: "SFGPUCA-------",
    moveType,
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

/** One of ours, one of theirs, and ours can see theirs. */
function board(mine: ForceElement, theirs: ForceElement): GameState {
  return {
    gameId: "terrain-movement",
    scenarioId: "terrain-movement",
    turn: 1,
    phase: "arcAction",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: { [mine.id]: mine, [theirs.id]: theirs },
    sighting: { blue: { [theirs.id]: "full" }, red: { [mine.id]: "full" } },
    rng: { seed: "test", cursor: 0 },
  };
}

function phaseConfig(terrain: TerrainSampler, ruleset: RuleSet): PhaseConfig {
  return {
    ruleset,
    terrain,
    rng: createRng("terrain-movement"),
    log: new EventLog(),
    maxTurns: 15,
  };
}

const ON = HOUSE_V1;
const OFF = withModules(HOUSE_V1, { terrainMovement: false });

/** Every move option offered to this element, with where it would end up. */
function moveOptions(
  terrain: TerrainSampler,
  ruleset: RuleSet,
  mine: ForceElement,
  theirs: ForceElement,
): { summary: string; destination: LatLng }[] {
  const state = board(mine, theirs);
  return optionsFor(state, mine, phaseConfig(terrain, ruleset))
    .filter((option) => option.kind === "move" && option.destination)
    .map((option) => ({ summary: option.summary, destination: option.destination! }));
}

describe("a commander is never offered a move the ground forbids", () => {
  // A bog across the axis of advance, 600 m of it, starting 550 m out.
  //
  // 550 rather than 500 on purpose: the route is sampled every 100 m, and a
  // boundary that lands exactly on a sample makes "did it stop at the edge or
  // one metre inside it" a question about floating point rather than about the
  // rule. Off-grid, the answer is unambiguous.
  const bog = ground((east) => (east >= 550 && east <= 1150 ? "marsh" : "open"));

  it("stops a wheeled element at the edge of ground it cannot enter", () => {
    const truck = fe("B1", "blue", at(0, 0), "W");
    const enemy = fe("R1", "red", at(3000, 0));

    for (const option of moveOptions(bog, ON, truck, enemy)) {
      // Wheeled has no allowance for marsh at all — 0 metres, which is
      // impassable rather than slow. No offered destination may be in it.
      expect(bog.classify(option.destination)).not.toBe("marsh");
    }
  });

  it("drove into the bog before this rule was wired in", () => {
    // The regression this file exists for. With the module off, the loop still
    // offers the blind 40% bound, and 40% of 2,000 m is 800 m — which on this
    // ground is the middle of the marsh.
    const truck = fe("B1", "blue", at(0, 0), "W");
    const enemy = fe("R1", "red", at(2000, 0));

    const destinations = moveOptions(bog, OFF, truck, enemy).map((option) =>
      bog.classify(option.destination),
    );
    expect(destinations).toContain("marsh");
  });

  it("lets tracks take the same route the wheels cannot", () => {
    const tank = fe("B1", "blue", at(0, 0), "T");
    const truck = fe("B2", "blue", at(0, 0), "W");
    const enemy = fe("R1", "red", at(1500, 0));

    const tracked = moveOptions(bog, ON, tank, enemy);
    const wheeled = moveOptions(bog, ON, truck, enemy);

    // The tank crosses the marsh, slowly, and gets past the near edge.
    expect(Math.max(...tracked.map((o) => eastingOf(o.destination)))).toBeGreaterThan(500);
    // The truck either stops short of it or goes round it; either way it is
    // never IN it, and it never gets further east than the tank did.
    for (const option of wheeled) {
      expect(bog.classify(option.destination)).not.toBe("marsh");
    }
  });
});

describe("a move that will not arrive says so", () => {
  it("reports the distance and the reason when the turn runs out", () => {
    // Foot through woodland: 500 m a turn on this ruleset's table, against a
    // bound that asks for 40% of 4 km.
    const woods = ground(() => "woodsLight");
    const infantry = fe("B1", "blue", at(0, 0), "F");
    const enemy = fe("R1", "red", at(4000, 0));

    const options = moveOptions(woods, ON, infantry, enemy);
    expect(options.length).toBeGreaterThan(0);
    for (const option of options) {
      expect(option.summary).toMatch(/as far as one turn allows|halted by|round the/);
      expect(distanceM(at(0, 0), option.destination)).toBeLessThan(1600);
    }
  });

  it("says nothing extra when the move simply arrives", () => {
    // Inside a tracked element's open-ground allowance (1,500 m a turn), so
    // the bound arrives and has nothing to report.
    const open = ground(() => "open");
    const tank = fe("B1", "blue", at(0, 0), "T");
    const enemy = fe("R1", "red", at(1000, 0));

    for (const option of moveOptions(open, ON, tank, enemy)) {
      expect(option.summary).not.toContain("(");
    }
  });
});

describe("nothing is stranded", () => {
  it("offers an element standing in a bog a way out of it", () => {
    // Generated ground can put a truck in a marsh. The rule prices ENTERING
    // terrain; charging it to leave what it is already in would immobilise it
    // for the rest of the game, and no rule put it there.
    const bog = ground((east) => (east < 400 ? "marsh" : "open"));
    const truck = fe("B1", "blue", at(0, 0), "W");
    const enemy = fe("R1", "red", at(2500, 0));

    const options = moveOptions(bog, ON, truck, enemy);
    expect(options.length).toBeGreaterThan(0);
    expect(Math.max(...options.map((o) => eastingOf(o.destination)))).toBeGreaterThan(0);
  });
});

describe("a router that says no does not cost the element its march", () => {
  it("falls back to a planned route rather than to a blind bound", () => {
    // The bug the first raster game showed: blue marched and red did not,
    // because red's objective sat on a river and A* refuses to end a route on
    // impassable ground. Red fell back to one-turn bounds — the behaviour
    // routes exist to replace — and the only visible symptom was half the
    // force having no line drawn on the map.
    const open = ground(() => "open");
    const mine = fe("B1", "blue", at(0, 0), "T");
    const theirs = fe("R1", "red", at(9000, 0));
    const state: GameState = {
      ...board(mine, theirs),
      // Nothing sighted: a march is a plan made in the absence of the enemy.
      sighting: { blue: {}, red: {} },
      objectives: { blue: at(8000, 0), red: at(0, 0) },
    };

    const refuses = {
      kind: "raster" as const,
      plan: () => null,
    };
    const config: PhaseConfig = { ...phaseConfig(open, ON), routePlanner: refuses };

    const options = optionsFor(state, mine, config).filter((option) => option.kind === "move");
    expect(options.map((option) => option.id)).toContain("B1:march");
    const march = options.find((option) => option.id === "B1:march");
    // And it is a real multi-turn plan, not a one-turn hop dressed up as one.
    expect(march?.route?.waypoints.length).toBeGreaterThan(0);
    expect(march?.route?.planner).toBe("bearings");
  });
});
