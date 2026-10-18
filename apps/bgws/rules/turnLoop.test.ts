import { describe, expect, it } from "vitest";

import { flatTerrain } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import { applyEffects, clearAllMarkers } from "./apply";
import { oddsColumnIndex, resolveAssault } from "./assault";
import { firstOptionCommander, heuristicCommander } from "./commander";
import { createRng } from "./dice";
import { EventLog, decisionBreadth } from "./events";
import { HOUSE_V1, withModules } from "./ruleset";
import { runGame, runTurn, type GameConfig } from "./turnLoop";

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

/** Two sides, a kilometre apart, in sight of each other. */
function skirmish(): GameState {
  const blue = [
    fe({ id: "B1", side: "blue", position: { lat: 54.71, lng: 20.51 } }),
    fe({ id: "B2", side: "blue", position: { lat: 54.712, lng: 20.51 } }),
  ];
  const red = [
    fe({ id: "R1", side: "red", position: { lat: 54.72, lng: 20.52 } }),
    fe({ id: "R2", side: "red", position: { lat: 54.722, lng: 20.52 } }),
  ];
  return {
    gameId: "test",
    scenarioId: "skirmish",
    turn: 1,
    phase: "command",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries([...blue, ...red].map((f) => [f.id, f])),
    sighting: { blue: {}, red: {} },
    rng: { seed: "test", cursor: 0 },
  };
}

function config(seed: string, overrides: Partial<GameConfig> = {}): GameConfig {
  const rng = createRng(seed);
  return {
    ruleset: HOUSE_V1,
    terrain: flatTerrain(),
    commanders: {
      blue: heuristicCommander("blue", rng),
      red: heuristicCommander("red", rng),
    },
    rng,
    log: new EventLog(),
    maxTurns: 15,
    ...overrides,
  };
}

describe("applying effects", () => {
  it("floors combat strength at zero", () => {
    // A negative strength would quietly invert every odds ratio it appears in.
    const state = skirmish();
    const next = applyEffects(state, [{ kind: "combatStrength", feId: "B1", delta: -99 }]);
    expect(next.forceElements.B1.combatStrength).toBe(0);
  });

  it("leaves the original state untouched", () => {
    const state = skirmish();
    applyEffects(state, [{ kind: "combatStrength", feId: "B1", delta: -3 }]);
    expect(state.forceElements.B1.combatStrength).toBe(8);
  });

  it("clears every marker at clean-up", () => {
    const state = applyEffects(skirmish(), [
      { kind: "marker", feId: "B1", marker: "fired", added: true },
    ]);
    expect(clearAllMarkers(state).forceElements.B1.markers).toEqual([]);
  });
});

describe("assault odds", () => {
  it("picks the column the ratio reaches, not the one above it", () => {
    expect(HOUSE_V1.assault.oddsColumns[oddsColumnIndex(HOUSE_V1, 2.9)]).toBe(2);
    expect(HOUSE_V1.assault.oddsColumns[oddsColumnIndex(HOUSE_V1, 3)]).toBe(3);
  });

  it("does not resolve hopeless odds more finely than the table can", () => {
    // One-to-four is not meaningfully worse than one-to-two; the lowest column
    // catches both rather than pretending to a precision it does not have.
    expect(oddsColumnIndex(HOUSE_V1, 0.25)).toBe(0);
    expect(oddsColumnIndex(HOUSE_V1, 0.5)).toBe(0);
  });

  it("favours the attacker as the odds lengthen", () => {
    const strong = resolveAssault(
      [fe({ id: "A", side: "blue", combatStrength: 24 })],
      [fe({ id: "D", side: "red", combatStrength: 4 })],
      {},
      HOUSE_V1,
      createRng("assault"),
      1,
      "arcAction",
    );
    const even = resolveAssault(
      [fe({ id: "A", side: "blue", combatStrength: 4 })],
      [fe({ id: "D", side: "red", combatStrength: 4 })],
      {},
      HOUSE_V1,
      createRng("assault"),
      1,
      "arcAction",
    );
    expect(strong.event.total!).toBeGreaterThan(even.event.total!);
  });

  it("treats an undefended position as unopposed rather than dividing by zero", () => {
    const outcome = resolveAssault(
      [fe({ id: "A", side: "blue" })],
      [fe({ id: "D", side: "red", combatStrength: 0 })],
      {},
      HOUSE_V1,
      createRng("empty"),
      1,
      "arcAction",
    );
    expect(outcome.event.table).toContain("unopposed");
    expect(outcome.result).toBe("defenderBreaks");
  });

  it("costs both sides in a melee", () => {
    const outcome = resolveAssault(
      [fe({ id: "A", side: "blue", combatStrength: 5 })],
      [fe({ id: "D", side: "red", combatStrength: 5 })],
      {},
      { ...HOUSE_V1, assault: { ...HOUSE_V1.assault, defenderBreaksAt: 99, attackRepulsedAt: -99 } },
      createRng("melee"),
      1,
      "arcAction",
    );
    expect(outcome.result).toBe("melee");
    expect(outcome.effects.filter((e) => e.kind === "combatStrength")).toHaveLength(2);
  });
});

describe("a turn", () => {
  it("runs, and leaves nobody holding a marker", () => {
    // Markers are a within-turn thing. One surviving into the next turn would
    // silently stop a unit acting.
    return runTurn(skirmish(), config("turn")).then((next) => {
      expect(next.turn).toBe(2);
      for (const unit of Object.values(next.forceElements)) {
        expect(unit.markers).toEqual([]);
      }
    });
  });

  it("records who decided what, with the options they had", () => {
    const cfg = config("decisions");
    return runTurn(skirmish(), cfg).then(() => {
      const decisions = cfg.log.decisions();
      expect(decisions.length).toBeGreaterThan(0);
      expect(decisions[0].chosenBy).toBe("heuristic");
      expect(decisions[0].options.length).toBeGreaterThan(0);
      // The chosen option must be one that was offered — the engine, not the
      // commander, decides what is legal.
      for (const decision of decisions) {
        expect(decision.options.map((o) => o.id)).toContain(decision.chosenId);
      }
    });
  });

  it("never offers an action to a unit that has already acted", () => {
    const cfg = config("markers");
    return runTurn(skirmish(), cfg).then(() => {
      const perTurn = new Map<string, number>();
      for (const decision of cfg.log.decisions()) {
        const actor = decision.actorId ?? "none";
        perTurn.set(actor, (perTurn.get(actor) ?? 0) + 1);
      }
      // Each force element gets at most one activation per turn.
      for (const count of perTurn.values()) expect(count).toBeLessThanOrEqual(1);
    });
  });
});

describe("a whole game, AI against AI", () => {
  it("reaches a conclusion", async () => {
    const outcome = await runGame(skirmish(), config("game-1"));
    expect(outcome.turns).toBeGreaterThan(0);
    expect(["annihilation", "turnLimit"]).toContain(outcome.reason);
  });

  it("replays exactly from the same seed", async () => {
    // The property the whole experiment harness rests on.
    const a = await runGame(skirmish(), config("replay"));
    const b = await runGame(skirmish(), config("replay"));
    expect(a.winner).toBe(b.winner);
    expect(a.turns).toBe(b.turns);
    expect(JSON.stringify(a.state.forceElements)).toBe(JSON.stringify(b.state.forceElements));
  });

  it("diverges on a different seed", async () => {
    const a = await runGame(skirmish(), config("seed-a"));
    const b = await runGame(skirmish(), config("seed-b"));
    const same =
      JSON.stringify(a.state.forceElements) === JSON.stringify(b.state.forceElements);
    expect(same).toBe(false);
  });

  it("produces commanders with real choices to make", async () => {
    // A game whose decision points all have one option is a cutscene. This is
    // the measure that says the rules produce agency.
    const cfg = config("agency");
    await runGame(skirmish(), cfg);
    const breadth = decisionBreadth(cfg.log);
    expect(breadth.total).toBeGreaterThan(0);
    expect(breadth.withRealChoice).toBeGreaterThan(0);
    expect(breadth.meanOptions).toBeGreaterThan(1);
  });

  it("lets two rulesets be compared on identical dice", async () => {
    // The harness in miniature: same scenario, same seed, one module flipped.
    const withPartial = withModules(HOUSE_V1, { partialSighting: true });
    const a = await runGame(skirmish(), config("compare"));
    const b = await runGame(skirmish(), { ...config("compare"), ruleset: withPartial });
    expect(a.state.gameId).toBe(b.state.gameId);
    // Both must complete; whether they differ is the experiment's finding,
    // not something a test should assert.
    expect(a.turns).toBeGreaterThan(0);
    expect(b.turns).toBeGreaterThan(0);
  });

  it("runs with a different commander without the rules noticing", async () => {
    const rng = createRng("mixed");
    const outcome = await runGame(
      skirmish(),
      config("mixed", {
        commanders: {
          blue: heuristicCommander("blue", rng),
          red: firstOptionCommander("red"),
        },
      }),
    );
    expect(outcome.turns).toBeGreaterThan(0);
  });
});
