import { describe, expect, it } from "vitest";

import type { ForceElement, Side } from "../lib/state";
import { createRng } from "./dice";
import { EventLog, decisionBreadth, modifierUsage } from "./events";
import {
  resolveDirectFire,
  resolveInitiative,
  resolveMoraleCheck,
  resolveSighting,
} from "./resolvers";
import { HOUSE_V1, fireColumnFor, fireResultFor, withModules, type FireResult } from "./ruleset";

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

describe("the fire table", () => {
  it("puts a stronger force on a better column", () => {
    expect(fireColumnFor(HOUSE_V1, 2).label).toBe("CS 1-2");
    expect(fireColumnFor(HOUSE_V1, 8).label).toBe("CS 6-9");
    expect(fireColumnFor(HOUSE_V1, 40).label).toBe("CS 16+");
  });

  it("rewards massing — the same roll does more on a higher column", () => {
    // If concentration of force does not pay, the game teaches the wrong
    // lesson. This is the assertion that keeps the ladder honest.
    const weak = fireColumnFor(HOUSE_V1, 2);
    const strong = fireColumnFor(HOUSE_V1, 20);

    // Asserted as an ORDERING rather than two literal results. The literals
    // changed the moment the table was recalibrated for game length, and a
    // test that breaks on every tuning pass gets deleted rather than read —
    // taking the principle it was protecting with it.
    const severity: FireResult[] = ["miss", "suppress", "oneHit", "twoHits", "threeHits"];
    const rank = (result: FireResult) => severity.indexOf(result);

    for (const roll of [7, 8, 9, 10]) {
      expect(
        rank(fireResultFor(strong, roll)),
        `roll ${roll}: massed fire must not do less than weak fire`,
      ).toBeGreaterThanOrEqual(rank(fireResultFor(weak, roll)));
    }
    // And strictly better somewhere, or the ladder is flat.
    expect(rank(fireResultFor(strong, 8))).toBeGreaterThan(rank(fireResultFor(weak, 8)));
  });
});

describe("direct fire", () => {
  const rng = () => createRng("fire-test");

  it("names every modifier it applied", () => {
    // An unexplained result is an unarguable one. Each modifier carries the
    // name it will be shown under in the turn log.
    const { event } = resolveDirectFire(
      [fe({ id: "A", side: "blue" })],
      fe({ id: "B", side: "red", markers: ["moved"] }),
      { targetInCover: true, smoke: true, rangeM: 2500, maxRangeM: 3000 },
      HOUSE_V1,
      rng(),
      1,
      "arcAction",
    );
    const sources = event.modifiers.map((m) => m.source).sort();
    expect(sources).toEqual(["longRange", "smoke", "targetInCover", "targetMoved"]);
  });

  it("takes the WORST firer's state for the whole group", () => {
    // Otherwise a commander stacks a suppressed unit's strength in for free,
    // and combined fire stops being a decision.
    const good = fe({ id: "A", side: "blue" });
    const shaken = fe({ id: "B", side: "blue", morale: "suppressed2" });
    const { event } = resolveDirectFire(
      [good, shaken],
      fe({ id: "T", side: "red" }),
      {},
      HOUSE_V1,
      rng(),
      1,
      "arcAction",
    );
    expect(event.modifiers.map((m) => m.source)).toContain("firerSuppressed");
  });

  it("sums combat strength across firers", () => {
    const { event } = resolveDirectFire(
      [fe({ id: "A", side: "blue", combatStrength: 6 }), fe({ id: "B", side: "blue", combatStrength: 6 })],
      fe({ id: "T", side: "red" }),
      {},
      HOUSE_V1,
      rng(),
      1,
      "arcAction",
    );
    // 12 combined, which is the CS 10-15 column rather than either firer's own.
    expect(event.table).toBe("fire:CS 10-15");
  });

  it("turns hits into strength loss and a shaken unit", () => {
    // A ruleset with certain hits, to test the effects rather than the dice.
    const certain = {
      ...HOUSE_V1,
      fireColumns: [
        { label: "always", minCombatStrength: 0, suppressAt: -99, oneHitAt: -99, twoHitsAt: 99, threeHitsAt: 99 },
      ],
    };
    const target = fe({ id: "T", side: "red", combatStrength: 4 });
    const { effects } = resolveDirectFire(
      [fe({ id: "A", side: "blue" })],
      target,
      {},
      certain,
      rng(),
      1,
      "arcAction",
    );
    // Taken from the ruleset, not written in: strength per hit is a tuning
    // lever and this test is about the effect being APPLIED, not its size.
    expect(effects).toContainEqual({
      kind: "combatStrength",
      feId: "T",
      delta: -certain.lethality.strengthPerHit,
    });
    expect(effects.some((e) => e.kind === "morale")).toBe(true);
    expect(effects.some((e) => e.kind === "eliminated")).toBe(false);
  });

  it("eliminates a force element whose strength reaches zero", () => {
    const certain = {
      ...HOUSE_V1,
      fireColumns: [
        { label: "always", minCombatStrength: 0, suppressAt: -99, oneHitAt: -99, twoHitsAt: 99, threeHitsAt: 99 },
      ],
    };
    const { effects } = resolveDirectFire(
      [fe({ id: "A", side: "blue" })],
      fe({ id: "T", side: "red", combatStrength: 1 }),
      {},
      certain,
      rng(),
      1,
      "arcAction",
    );
    expect(effects).toContainEqual({ kind: "eliminated", feId: "T" });
  });

  it("resolves identically from the same seed", () => {
    const once = resolveDirectFire([fe({ id: "A", side: "blue" })], fe({ id: "T", side: "red" }), {}, HOUSE_V1, createRng("same"), 1, "arcAction");
    const twice = resolveDirectFire([fe({ id: "A", side: "blue" })], fe({ id: "T", side: "red" }), {}, HOUSE_V1, createRng("same"), 1, "arcAction");
    expect(once.event.total).toBe(twice.event.total);
    expect(once.event.result).toBe(twice.event.result);
  });
});

describe("morale", () => {
  it("makes a better unit steadier", () => {
    const good = resolveMoraleCheck(fe({ id: "A", side: "blue", troopQuality: 8 }), {}, HOUSE_V1, createRng("m"), 1, "command");
    const poor = resolveMoraleCheck(fe({ id: "A", side: "blue", troopQuality: 1 }), {}, HOUSE_V1, createRng("m"), 1, "command");
    expect(good.event.total!).toBeGreaterThan(poor.event.total!);
  });

  it("penalises a unit that has taken losses", () => {
    const fresh = resolveMoraleCheck(fe({ id: "A", side: "blue" }), {}, HOUSE_V1, createRng("m"), 1, "command");
    const mauled = resolveMoraleCheck(
      fe({ id: "A", side: "blue", combatStrength: 3, combatStrengthStart: 8 }),
      {},
      HOUSE_V1,
      createRng("m"),
      1,
      "command",
    );
    expect(mauled.event.total!).toBeLessThan(fresh.event.total!);
    expect(mauled.event.modifiers.map((m) => m.source)).toContain("strengthLost");
  });

  it("steadies a unit beside its HQ", () => {
    const alone = resolveMoraleCheck(fe({ id: "A", side: "blue" }), {}, HOUSE_V1, createRng("m"), 1, "command");
    const withHq = resolveMoraleCheck(fe({ id: "A", side: "blue" }), { hqPresent: true }, HOUSE_V1, createRng("m"), 1, "command");
    expect(withHq.event.total!).toBeGreaterThan(alone.event.total!);
  });
});

describe("sighting", () => {
  it("collapses partial contact to full when the module is off", () => {
    // The module switch has to work without line of sight knowing about it.
    const binary = withModules(HOUSE_V1, { partialSighting: false });
    const results = new Set<string>();
    for (let i = 0; i < 60; i++) {
      const { event } = resolveSighting(
        fe({ id: "O", side: "blue" }),
        fe({ id: "T", side: "red" }),
        {},
        binary,
        createRng(`s-${i}`),
        1,
        "arcAction",
        "blue",
      );
      results.add(event.result);
    }
    expect(results.has("partial")).toBe(false);
    expect(results.has("full")).toBe(true);
  });

  it("produces partial contacts when the module is on", () => {
    const threeState = withModules(HOUSE_V1, { partialSighting: true });
    const results = new Set<string>();
    for (let i = 0; i < 60; i++) {
      const { event } = resolveSighting(
        fe({ id: "O", side: "blue" }),
        fe({ id: "T", side: "red" }),
        {},
        threeState,
        createRng(`s-${i}`),
        1,
        "arcAction",
        "blue",
      );
      results.add(event.result);
    }
    expect(results.has("partial")).toBe(true);
  });

  it("is harder at range", () => {
    const near = fe({ id: "T", side: "red", position: { lat: 54.71, lng: 20.51 } });
    const far = fe({ id: "T", side: "red", position: { lat: 54.73, lng: 20.55 } });
    const observer = fe({ id: "O", side: "blue" });
    const a = resolveSighting(observer, near, {}, HOUSE_V1, createRng("r"), 1, "arcAction", "blue");
    const b = resolveSighting(observer, far, {}, HOUSE_V1, createRng("r"), 1, "arcAction", "blue");
    expect(b.event.total!).toBeLessThan(a.event.total!);
  });
});

describe("initiative", () => {
  it("penalises the side that lost force elements last turn", () => {
    // The transmission-advantage test that used to live here went with the
    // `transmissions` module: swept over 500 games across five force lists,
    // it changed 0% of decisions and 0% of outcomes, because every activation
    // cost both sides exactly one transmission so the difference was always
    // zero. Losses are what initiative actually turns on now.
    const heavyLosses = resolveInitiative(
      { blue: 0, red: 0 },
      { blue: 0, red: 3 },
      HOUSE_V1,
      createRng("init"),
      1,
    );
    expect(
      heavyLosses.event.modifiers.some((m) => m.source === "red:lossesLastTurn"),
    ).toBe(true);
  });

  it("ignores transmissions entirely when the module is off", () => {
    const result = resolveInitiative(
      { blue: 1, red: 9 },
      { blue: 0, red: 0 },
      HOUSE_V1,
      createRng("init"),
      2,
    );
    expect(result.event.modifiers.map((m) => m.source)).not.toContain(
      "blue:transmissionAdvantage",
    );
  });

  it("always produces a winner, including on a tie", () => {
    for (let i = 0; i < 40; i++) {
      const result = resolveInitiative({ blue: 3, red: 3 }, { blue: 0, red: 0 }, HOUSE_V1, createRng(`t-${i}`), 1);
      expect(["blue", "red"]).toContain(result.winner);
    }
  });
});

describe("the log as evidence", () => {
  it("counts which modifiers ever actually fired", () => {
    // The experiment harness's blunt instrument: a DRM that never appears is a
    // rule with nothing to say.
    const log = new EventLog();
    const { event } = resolveDirectFire(
      [fe({ id: "A", side: "blue" })],
      fe({ id: "T", side: "red" }),
      { smoke: true },
      HOUSE_V1,
      createRng("usage"),
      1,
      "arcAction",
    );
    log.append(event);

    const usage = modifierUsage(log);
    expect(usage.smoke.count).toBe(1);
    expect(usage.flank).toBeUndefined();
  });

  it("measures whether commanders had a real choice", () => {
    const log = new EventLog();
    log.append({
      type: "decision",
      turn: 1,
      phase: "arcAction",
      rulesetId: HOUSE_V1.id,
      side: "blue",
      question: "activate which group?",
      options: [
        { id: "g1", summary: "1 Tp" },
        { id: "g2", summary: "2 Tp" },
      ],
      chosenId: "g1",
      chosenBy: "heuristic",
    });
    log.append({
      type: "decision",
      turn: 1,
      phase: "arcAction",
      rulesetId: HOUSE_V1.id,
      side: "red",
      question: "activate which group?",
      options: [{ id: "only", summary: "no alternative" }],
      chosenId: "only",
      chosenBy: "heuristic",
    });

    const breadth = decisionBreadth(log);
    expect(breadth.total).toBe(2);
    // A decision point with one option is not a decision.
    expect(breadth.withRealChoice).toBe(1);
    expect(breadth.meanOptions).toBe(1.5);
  });
});
