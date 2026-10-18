import { describe, expect, it } from "vitest";

import {
  MEETING_ENGAGEMENT_V1,
  PLATFORM_SNAPSHOT,
  SYMMETRIC_CONTROL_V1,
  TROOP_QUALITY,
  FORCE_LISTS,
} from "../rules/forceList";
import { HOUSE_V1, combatStrengthFor as rulesetCs, fireColumnFor, withModules } from "../rules/ruleset";
import {
  DEFAULT_ORIGIN,
  combatStrengthFor,
  offsetToLatLng,
  proxiesIn,
  scenarioFactory,
  startingStrength,
  toForceElement,
  toGameState,
  toGameStateFromPlaced,
} from "./forceBuilder";
import { activationBudget } from "../rules/turnLoop";
import { createRng } from "../rules/dice";
import { EventLog } from "../rules/events";
import { proceduralTerrain, STANDARD_GROUND } from "./proceduralTerrain";

describe("a hand-placed force can have a headquarters", () => {
  // ⚠ THE PLUMBING WAS COMPLETE AND NOTHING FED IT. `PlacedElement` has
  // carried `commandRating` since it was written and `toGameStateFromPlaced`
  // has always passed it through, but the placement brush set side, platform,
  // count and quality and nothing else. So every scenario built by hand was
  // leaderless, `activationBudget` sat on its floor of 2 for the whole game,
  // and `hqPresent` could never fire. Both mechanics were reachable only
  // through the scripted force lists.
  const capped = withModules(HOUSE_V1, { commandActivations: true });
  // activationBudget reads only the ruleset, but takes the whole PhaseConfig.
  const phase = {
    ruleset: capped,
    terrain: proceduralTerrain(STANDARD_GROUND),
    rng: createRng("hq"),
    log: new EventLog(),
    maxTurns: 12,
  };

  const place = (commandRating?: number) => [
    {
      id: "a",
      label: "A",
      side: "blue" as const,
      platform: "var_11_default",
      platformCount: 4,
      troopQuality: "regular" as const,
      position: DEFAULT_ORIGIN,
      commandRating,
    },
  ];

  it("gives a leaderless side only the floor", () => {
    const state = toGameStateFromPlaced(place(), capped);
    expect(activationBudget(state, "blue", phase)).toBe(
      capped.logistics.activationsWithoutHq,
    );
  });

  it("lifts the budget once one is placed", () => {
    const state = toGameStateFromPlaced(place(3), capped);
    expect(activationBudget(state, "blue", phase)).toBe(3);
    expect(activationBudget(state, "blue", phase)).toBeGreaterThan(
      capped.logistics.activationsWithoutHq,
    );
  });
});

describe("combat strength", () => {
  const challenger = PLATFORM_SNAPSHOT["var_11_default"];

  it("adds platform count rather than multiplying by it", () => {
    // THE BUG THIS TEST EXISTS FOR. csIndex * count gave a four-Challenger
    // troop a Combat Strength of 34, which saturates the top fire column so
    // every unit fires identically, and needs ~34 hits to kill so no game
    // ever reaches a decision. A full module sweep then reported all ten
    // mechanics as candidates for deletion. All ten readings were artefacts.
    const troop = combatStrengthFor(HOUSE_V1, challenger, 4);
    // 8.6 * 0.5 + 4 + 2 (heavy) = 10.3
    expect(troop).toBe(10);
    expect(troop).not.toBe(Math.round(challenger.csIndex * 4));
  });

  it("weights protection, so an IFV is not a tank", () => {
    // ⚠ REWRITTEN BECAUSE ITS PREMISE WAS FIXED UPSTREAM, AND IT WOULD HAVE
    // GONE GREEN WHILE TESTING NOTHING.
    //
    // It used to read the Warrior out of the snapshot and assert its csIndex
    // was within a point of the Challenger's — 8.5 against 8.6, the simulator
    // rating an IFV as a tank on the strength of a TOW borrowed from a
    // Kuwaiti export model — and then that the house rule pulled the two
    // apart. The curated profile rates the real FV510 at 4.3, so the second
    // assertion now holds on the raw index alone. The test would have kept
    // passing and stopped saying anything at all about the protection term.
    //
    // So the pair is synthetic and differs in EXACTLY ONE FIELD. That is the
    // only shape that can show the band is what does the work, and it cannot
    // be quietly invalidated by better input data later.
    const heavy = { ...challenger, protectionBand: "heavy" };
    const light = { ...challenger, protectionBand: "light" };
    expect(heavy.csIndex).toBe(light.csIndex);
    expect(combatStrengthFor(HOUSE_V1, heavy, 4)).toBeGreaterThan(
      combatStrengthFor(HOUSE_V1, light, 4),
    );
  });

  it("no longer needs the house rule to rescue the Warrior", () => {
    // The companion to the above, and deliberately a DATA test rather than a
    // rule test: it asserts the curated figures are sane, not that the
    // ruleset works. If a future curation pass rates an IFV as a tank again,
    // this is what should go red — not the test above.
    const warrior = PLATFORM_SNAPSHOT["var_27_default"];
    expect(warrior.protectionBand).toBe("light");
    expect(warrior.csIndex).toBeLessThan(challenger.csIndex / 1.5);
    // A Warrior platoon at 5 against a Challenger troop's 10.
    expect(combatStrengthFor(HOUSE_V1, challenger, 4)).toBeGreaterThanOrEqual(
      combatStrengthFor(HOUSE_V1, warrior, 4) + 4,
    );
  });

  it("puts a modern tank on a better fire column than a 1950s one", () => {
    // Staying power alone was not enough: CS 14 and CS 10 sat on the same
    // column, so forty years of tank design bought no firepower at all and
    // the generation probe stalled at 62%.
    //
    // ⚠ THE GAP IS NOW MADE HONESTLY AND IS NARROWER FOR IT. It used to come
    // from an M1A2 hand-pinned at csIndex 9.4 and `very_heavy` — a rating the
    // curated index cannot produce and a band that appears in none of its 579
    // rows. The Abrams is now the sourced 8.6/`heavy`, and the separation
    // comes from correcting the OTHER side: a Type 59 with 200 mm of hull was
    // banded `heavy`, which the curated pipeline's own rule reserves for
    // 300 mm and above. Troop CS 10 against 8.
    //
    // If a future change collapses these onto one column, the fix is the fire
    // table or the band thresholds. It is NOT a larger csIndex typed into the
    // snapshot, which is how this was papered over the first time.
    const modern = PLATFORM_SNAPSHOT["var_1_default"];
    const old = PLATFORM_SNAPSHOT["var_20_default"];
    expect(modern.protectionBand).toBe("heavy");
    expect(old.protectionBand).toBe("medium");
    const modernColumn = fireColumnFor(HOUSE_V1, combatStrengthFor(HOUSE_V1, modern, 4));
    const oldColumn = fireColumnFor(HOUSE_V1, combatStrengthFor(HOUSE_V1, old, 4));
    expect(modernColumn.label).not.toBe(oldColumn.label);
    expect(modernColumn.oneHitAt).toBeLessThan(oldColumn.oneHitAt);
  });

  it("ignores a protection band it does not recognise", () => {
    // A new band upstream should make a unit slightly wrong, not crash a game.
    const odd = { ...challenger, protectionBand: "unobtainium" };
    expect(() => combatStrengthFor(HOUSE_V1, odd, 4)).not.toThrow();
  });

  it("keeps a sub-unit inside the fire table's range", () => {
    // The top column is "CS 16+". Anything above it is off the end of the
    // table and indistinguishable from anything else up there.
    for (const count of [1, 2, 3, 4, 8, 16, 40]) {
      const strength = combatStrengthFor(HOUSE_V1, challenger, count);
      expect(strength).toBeLessThanOrEqual(HOUSE_V1.combatStrength.max);
      expect(strength).toBeGreaterThanOrEqual(HOUSE_V1.combatStrength.min);
    }
  });

  it("still lets count matter", () => {
    // A troop of four must out-gun a platoon of three, or the granularity
    // decision bought nothing.
    expect(combatStrengthFor(HOUSE_V1, challenger, 4)).toBeGreaterThan(
      combatStrengthFor(HOUSE_V1, challenger, 3),
    );
  });

  it("lands a four-platform troop mid-table, not on the end column", () => {
    const column = fireColumnFor(HOUSE_V1, combatStrengthFor(HOUSE_V1, challenger, 4));
    expect(column.label).toBe("CS 10-15");
  });

  it("delegates to the ruleset, so the mapping is swappable", () => {
    // The point of moving it into the RuleSet: a different mapping is a
    // different ruleset, and the harness can measure the difference.
    const heavier = { ...HOUSE_V1, combatStrength: { ...HOUSE_V1.combatStrength, countWeight: 2 } };
    expect(combatStrengthFor(heavier, challenger, 4)).toBeGreaterThan(
      combatStrengthFor(HOUSE_V1, challenger, 4),
    );
    expect(combatStrengthFor(HOUSE_V1, challenger, 4)).toBe(
      rulesetCs(HOUSE_V1, challenger.csIndex, 4, challenger.protectionBand),
    );
  });
});

describe("building force elements", () => {
  const spec = MEETING_ENGAGEMENT_V1.elements[0];

  it("takes troop quality from the force list, never from equipment", () => {
    const fe = toForceElement(spec, DEFAULT_ORIGIN, HOUSE_V1);
    expect(fe.troopQuality).toBe(TROOP_QUALITY[spec.troopQuality]);
    // Not the old hardcoded 4 that applied to every unit on the board.
    expect(fe.troopQuality).toBe(TROOP_QUALITY.veteran);
  });

  it("carries the real per-capability weapon ranges", () => {
    const fe = toForceElement(spec, DEFAULT_ORIGIN, HOUSE_V1);
    const ranges = fe.capabilities.map((c) => c.maxRangeM);
    // Under the old flat 3,000 m these were all identical, so no rule about
    // range could possibly have shown an effect.
    expect(new Set(ranges).size).toBeGreaterThan(1);
    expect(fe.capabilities.find((c) => c.kind === "apers")?.maxRangeM).toBe(2000);
    expect(fe.capabilities.find((c) => c.kind === "atk")?.maxRangeM).toBe(3000);
  });

  it("copies capabilities so one game cannot edit the next game's equipment", () => {
    const a = toForceElement(spec, DEFAULT_ORIGIN, HOUSE_V1);
    a.capabilities[0].maxRangeM = 1;
    const b = toForceElement(spec, DEFAULT_ORIGIN, HOUSE_V1);
    expect(b.capabilities[0].maxRangeM).not.toBe(1);
  });

  it("throws loudly on an unknown platform", () => {
    // The alternative is an FE with no capabilities that loses every
    // engagement for a reason nobody would ever find.
    expect(() =>
      toForceElement({ ...spec, platform: "tankmodels/not_real" }, DEFAULT_ORIGIN, HOUSE_V1),
    ).toThrow(/PLATFORM_SNAPSHOT/);
  });

  it("starts at full strength and good morale", () => {
    const fe = toForceElement(spec, DEFAULT_ORIGIN, HOUSE_V1);
    expect(fe.combatStrength).toBe(fe.combatStrengthStart);
    expect(fe.morale).toBe("good");
  });
});

describe("positions", () => {
  it("puts north of the origin at a higher latitude", () => {
    expect(offsetToLatLng(DEFAULT_ORIGIN, 0, 1000).lat).toBeGreaterThan(DEFAULT_ORIGIN.lat);
  });

  it("converts a metre offset to within a metre", () => {
    const moved = offsetToLatLng(DEFAULT_ORIGIN, 0, 1600);
    const metres = (moved.lat - DEFAULT_ORIGIN.lat) * 111_320;
    expect(Math.abs(metres - 1600)).toBeLessThan(1);
  });

  it("separates the two sides by their deployment offsets", () => {
    const state = toGameState(SYMMETRIC_CONTROL_V1, HOUSE_V1);
    const blue = state.forceElements["blue-1"];
    const red = state.forceElements["red-1"];
    const metres = (red.position.lat - blue.position.lat) * 111_320;
    expect(Math.round(metres)).toBe(1600);
  });
});

describe("the scenario factory", () => {
  it("returns a FRESH state each call", () => {
    // A shared state would let game two inherit game one's casualties, and a
    // batch would report a steadily collapsing force as a finding.
    const factory = scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1);
    const first = factory();
    first.forceElements["blue-1"].combatStrength = 0;
    expect(factory().forceElements["blue-1"].combatStrength).toBeGreaterThan(0);
  });

  it("is unaffected by module toggles, which must not change the force", () => {
    // moduleImpact runs the same scenario with a module off and on. If the
    // force differed between arms the comparison would be meaningless.
    const on = toGameState(SYMMETRIC_CONTROL_V1, withModules(HOUSE_V1, { dummies: true }));
    const off = toGameState(SYMMETRIC_CONTROL_V1, withModules(HOUSE_V1, { dummies: false }));
    expect(startingStrengthOf(on)).toEqual(startingStrengthOf(off));
  });
});

function startingStrengthOf(state: ReturnType<typeof toGameState>): number {
  return Object.values(state.forceElements).reduce((sum, fe) => sum + fe.combatStrength, 0);
}

describe("the force lists themselves", () => {
  it("every element names a platform that exists", () => {
    for (const list of Object.values(FORCE_LISTS)) {
      for (const spec of list.elements) {
        expect(PLATFORM_SNAPSHOT[spec.platform], `${list.id} / ${spec.id}`).toBeDefined();
      }
    }
  });

  it("every element has a unique id", () => {
    for (const list of Object.values(FORCE_LISTS)) {
      const ids = list.elements.map((spec) => spec.id);
      expect(new Set(ids).size, list.id).toBe(ids.length);
    }
  });

  it("every list has both sides represented", () => {
    for (const list of Object.values(FORCE_LISTS)) {
      const sides = new Set(list.elements.map((spec) => spec.side));
      expect(sides, list.id).toEqual(new Set(["blue", "red"]));
    }
  });

  it("the control is exactly symmetric", () => {
    // Its whole purpose. If a module moves the win rate here, it moved it.
    const strength = startingStrength(SYMMETRIC_CONTROL_V1, HOUSE_V1);
    expect(strength.blue).toBe(strength.red);
  });

  it("the meeting engagement is balanced to within a tenth", () => {
    // Not symmetric, but close enough that a module comparison is not just
    // measuring the imbalance. British troops are four platforms and Soviet
    // platoons three, which is doctrinally right and experimentally awful.
    const strength = startingStrength(MEETING_ENGAGEMENT_V1, HOUSE_V1);
    const ratio = Math.abs(strength.blue - strength.red) / strength.blue;
    expect(ratio).toBeLessThan(0.1);
  });

  it("fields no unnamed stand-ins", () => {
    // ⚠ THIS USED TO REQUIRE AT LEAST ONE PROXY, AND THAT REQUIREMENT IS NOW
    // BACKWARDS. The meeting engagement carried two: an Italian export T-72M1
    // standing in for a T-72B3, and a Swedish trials T-80U standing in for a
    // Russian one, both because the simulator modelled no better. The curated
    // tables model both, so the stand-ins are gone and `> 0` fails on a list
    // that has got MORE honest, not less.
    //
    // What the test was really protecting is that nothing pretends to be a
    // vehicle it is not, so that is what it now asserts: a platform may be
    // sourced, or declared, or a named proxy, and it must be one of them.
    for (const spec of MEETING_ENGAGEMENT_V1.elements) {
      const platform = PLATFORM_SNAPSHOT[spec.platform];
      const provenance =
        platform.sourcedFrom ?? platform.valuesDeclared ?? platform.proxyFor;
      expect(provenance, `${spec.label} (${platform.displayName})`).toBeTruthy();
    }
    for (const proxy of proxiesIn(MEETING_ENGAGEMENT_V1)) {
      expect(proxy.proxyFor).toBeTruthy();
    }
  });

  it("still detects a proxy when one is fielded", () => {
    // The companion to the above. With no proxy left in any real list,
    // `proxiesIn` would be entirely untested — and a helper that silently
    // returns nothing is indistinguishable from one that works. So it is
    // exercised against a list built to contain one.
    const list: typeof MEETING_ENGAGEMENT_V1 = {
      ...MEETING_ENGAGEMENT_V1,
      elements: [MEETING_ENGAGEMENT_V1.elements[0]],
    };
    expect(proxiesIn(list)).toEqual([]);

    const proxied = { ...PLATFORM_SNAPSHOT["var_11_default"], proxyFor: "something else" };
    const restore = PLATFORM_SNAPSHOT["var_11_default"];
    (PLATFORM_SNAPSHOT as Record<string, typeof proxied>)["var_11_default"] = proxied;
    try {
      const found = proxiesIn(list);
      expect(found).toHaveLength(1);
      expect(found[0].proxyFor).toBe("something else");
    } finally {
      (PLATFORM_SNAPSHOT as Record<string, typeof restore>)["var_11_default"] = restore;
    }
  });

  it("the control fields no proxies", () => {
    expect(proxiesIn(SYMMETRIC_CONTROL_V1)).toEqual([]);
  });

  it("carries the Combat Strength caveat, which caps what a run can conclude", () => {
    // ⚠ THE CAVEAT NARROWED BECAUSE THE DEFECT NARROWED, AND THE TEST HAD TO
    // FOLLOW. It used to look for "COMBAT STRENGTH BARELY DISCRIMINATES",
    // which was fair when the index ran 8.0 to 9.4 across everything and put
    // a Warrior within a tenth of a Challenger. The Warrior is now 4.3 and
    // the index no longer saturates, so that sentence would be false and a
    // test demanding a false sentence is worse than no test.
    //
    // What survives is the narrower, still-true limit: between TANKS the
    // index is flat, so a null result on this list is not evidence that
    // equipment does not matter.
    const caveats = MEETING_ENGAGEMENT_V1.caveats.join(" ");
    expect(caveats).toMatch(/COMBAT STRENGTH DISCRIMINATES COARSELY AT THE TOP/);
    // And the finding most likely to mislead a reader now. The hole this
    // used to name — guns with no penetrationMm, so armour never bit — is
    // closed. What replaced it is the consequence: with armour biting, peer
    // tanks largely cannot kill each other and a penetration-on run measures
    // a stalemate. A reader who misses that will read "40 turns" as the
    // rules failing rather than as armour working.
    expect(caveats).toMatch(/stalemate/i);
    expect(caveats).toMatch(/40-turn cap/);
  });
});
