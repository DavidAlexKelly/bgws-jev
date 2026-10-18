import { describe, expect, it } from "vitest";

import type { ForceElement, GameState, Side } from "../lib/state";
import { firstOptionCommander } from "./commander";
import { HOUSE_V1 } from "./ruleset";
import {
  compare,
  declaredModifiers,
  describeBatch,
  moduleImpact,
  runBatch,
} from "./harness";

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

const scenario = (): GameState => ({
  gameId: "harness",
  scenarioId: "meeting-engagement",
  turn: 1,
  phase: "command",
  initiative: null,
  sides: {
    blue: { transmissions: 2, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    red: { transmissions: 6, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
  },
  forceElements: Object.fromEntries(
    [
      fe({ id: "B1", side: "blue", position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "B2", side: "blue", position: { lat: 54.711, lng: 20.512 } }),
      fe({ id: "R1", side: "red", position: { lat: 54.718, lng: 20.518 } }),
      fe({ id: "R2", side: "red", position: { lat: 54.719, lng: 20.52 } }),
    ].map((f) => [f.id, f]),
  ),
  sighting: { blue: {}, red: {} },
  rng: { seed: "unused", cursor: 0 },
});

const SEEDS = ["s1", "s2", "s3", "s4", "s5", "s6"];

describe("batch runs", () => {
  it("plays every seed to a conclusion", async () => {
    const result = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    expect(result.games).toHaveLength(SEEDS.length);
    expect(result.wins.blue + result.wins.red + result.wins.draw).toBe(SEEDS.length);
    expect(result.meanTurns).toBeGreaterThan(0);
  });

  it("is reproducible", async () => {
    const a = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    const b = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    expect(a.games.map((g) => g.winner)).toEqual(b.games.map((g) => g.winner));
    expect(a.games.map((g) => g.decisionTrace.join())).toEqual(
      b.games.map((g) => g.decisionTrace.join()),
    );
  });

  it("names the modifiers that never fired", async () => {
    // The cheapest possible finding: a rule that has never once applied.
    // `flank` is declared in the ruleset and nothing in the turn loop sets it,
    // so it should be reported rather than sitting there looking implemented.
    const result = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    expect(result.neverFired).toContain("flank");
    expect(declaredModifiers(HOUSE_V1)).toContain("flank");
  });

  it("reports the modifiers that did fire", async () => {
    const result = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    // Range always applies to a sighting attempt, so it is the one modifier
    // guaranteed to be exercised by any game at all.
    expect(result.modifierUsage.range).toBeGreaterThan(0);
  });

  it("summarises readably", async () => {
    const result = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    const text = describeBatch(result);
    expect(text).toContain(HOUSE_V1.id);
    expect(text).toContain("mean length");
  });
});

describe("comparing rulesets", () => {
  it("pairs games by seed and reports what changed", async () => {
    const a = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    const harsher = {
      ...HOUSE_V1,
      id: "harsher",
      fireColumns: HOUSE_V1.fireColumns.map((c) => ({
        ...c,
        oneHitAt: c.oneHitAt - 3,
        twoHitsAt: c.twoHitsAt - 3,
      })),
    };
    const b = await runBatch({ scenario, ruleset: harsher, seeds: SEEDS, maxTurns: 12 });

    const result = compare(a, b, "much deadlier fire");
    expect(result.verdict).toContain("much deadlier fire");
    // Making fire three points deadlier has to do SOMETHING to a game that is
    // decided by fire; if this ever reads zero, the comparison is broken.
    expect(
      result.outcomesChanged.length + result.decisionsChanged.length,
    ).toBeGreaterThan(0);
  });

  it("says plainly when a change did nothing", async () => {
    const a = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    const renamedOnly = { ...HOUSE_V1, id: "identical-but-renamed", name: "same numbers" };
    const b = await runBatch({ scenario, ruleset: renamedOnly, seeds: SEEDS, maxTurns: 12 });

    const result = compare(a, b, "no change at all");
    expect(result.outcomesChanged).toHaveLength(0);
    expect(result.decisionsChanged).toHaveLength(0);
    expect(result.verdict).toContain("candidate for deletion");
  });

  it("always carries the caveat about what the numbers mean", async () => {
    // A batch result read without this line is a claim about warfare, which it
    // is not. The caveat travels with the verdict rather than living in a
    // README nobody opens.
    const a = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    const result = compare(a, a, "self");
    expect(result.verdict).toContain("not about combat");
  });
});

describe("does a module earn its place", () => {
  it("measures one, and produces a verdict", async () => {
    const result = await moduleImpact("ammunition", {
      scenario,
      ruleset: HOUSE_V1,
      seeds: SEEDS,
      maxTurns: 12,
    });
    expect(result.verdict).toContain("ammunition");
    expect(result.a.rulesetId).not.toBe(result.b.rulesetId);
  });

  it("distinguishes changing a decision from changing an outcome", async () => {
    // The distinction the whole harness exists for: a rule can flip a winner
    // through the dice alone, and that is much weaker evidence than a rule
    // that makes a commander choose differently.
    const result = await moduleImpact("partialSighting", {
      scenario,
      ruleset: HOUSE_V1,
      seeds: SEEDS,
      maxTurns: 12,
    });
    const saysDecisions = result.verdict.includes("decisions changed in");
    const saysOutcomes = result.verdict.includes("outcomes in");
    expect(saysDecisions && saysOutcomes).toBe(true);
  });
});

describe("commanders are comparable", () => {
  it("swaps a commander without disturbing the dice", async () => {
    // Dice and decisions draw from separate streams, so a game played by a
    // different commander is still the same battle with the same luck. Without
    // this, comparing a model against the bot would measure the dice.
    const bot = await runBatch({ scenario, ruleset: HOUSE_V1, seeds: SEEDS, maxTurns: 12 });
    const trivial = await runBatch({
      scenario,
      ruleset: HOUSE_V1,
      seeds: SEEDS,
      maxTurns: 12,
      commander: (side) => firstOptionCommander(side),
    });

    expect(trivial.games).toHaveLength(SEEDS.length);
    // Different commanders should play differently; if the traces matched, the
    // commander would not be deciding anything.
    const sameTraces =
      bot.games.map((g) => g.decisionTrace.join()).join("#") ===
      trivial.games.map((g) => g.decisionTrace.join()).join("#");
    expect(sameTraces).toBe(false);
  });
});
