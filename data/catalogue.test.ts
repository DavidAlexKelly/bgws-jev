/**
 * THE CATALOGUE AND THE THINGS WE DECLARED.
 *
 * The game can field 819 of the 1,239 platforms in the L6 profiles. The other
 * 420 — every soft-skin, every foot platform, and 210 vehicles missing ranges
 * or a combat strength — are browsable and not playable, which is a fact
 * about the source rather than a gap in the code.
 *
 * These tests guard the two ways that arrangement can rot: a generated file
 * silently deleting the hand-declared platforms, and a force list naming a
 * platform that does not exist or cannot fight.
 */

import { describe, expect, it } from "vitest";

import { CATALOGUE_COVERAGE, CATALOGUE_PLATFORMS } from "./platformCatalogue.generated";
import { describePlayability, playabilityOf } from "./playability";
import { scenarioFactory } from "../lib/forceBuilder";
import { HOUSE_V1 } from "../rules/ruleset";
import {
  ALL_FORCE_LISTS,
  PINNED_PLATFORMS,
  PLATFORM_SNAPSHOT,
  symmetricListFrom,
} from "../rules/forceList";

describe("the gate", () => {
  it("passes a platform that can move, be shot at, fight and fire", () => {
    const result = playabilityOf({
      moveType: "T",
      targetClass: "armoured_vehicle",
      csIndex: 9,
      capabilities: [{ maxRangeM: 3000 }],
    });
    expect(result.playable).toBe(true);
    expect(describePlayability(result)).toBe("Playable");
  });

  it("names EVERY reason, not just the first", () => {
    // A greyed-out row in the Asset Explorer that says "no weapon ranges" is
    // a fact about the source. One with no reason is a bug report waiting to
    // be filed.
    const shell = playabilityOf({
      moveType: null,
      targetClass: null,
      csIndex: 0,
      capabilities: [],
    });
    expect(shell.playable).toBe(false);
    expect(shell.reasons).toEqual([
      "noMoveType",
      "noTargetClass",
      "noCombatStrength",
      "noWeaponRanges",
    ]);
    expect(describePlayability(shell)).toContain("could never fire a shot");
  });

  it("rejects the shape every foot platform in the catalogue has", () => {
    // All 76 of them: a name, an id, and nothing else.
    const foot = playabilityOf({
      moveType: "F",
      targetClass: "foot",
      csIndex: 0,
      capabilities: null,
    });
    expect(foot.reasons).toEqual(["noCombatStrength", "noWeaponRanges"]);
  });

  it("judges capability, not confidence", () => {
    // A low-confidence platform with real ranges is playable and its figures
    // are soft. A platform with no ranges is unplayable at any confidence.
    // Conflating them would hide the second behind the first.
    const soft = playabilityOf({
      moveType: "W",
      targetClass: "soft_skin",
      csIndex: 1,
      capabilities: [{ maxRangeM: 400 }],
    });
    expect(soft.playable).toBe(true);
  });
});

describe("the generated catalogue", () => {
  it("only contains platforms that pass the gate", () => {
    for (const platform of Object.values(CATALOGUE_PLATFORMS)) {
      expect(playabilityOf(platform).playable, platform.displayName).toBe(true);
    }
  });

  it("keys every entry by its own asset id", () => {
    for (const [key, platform] of Object.entries(CATALOGUE_PLATFORMS)) {
      expect(platform.assetId).toBe(key);
    }
  });

  it("never invents a weapon — a missile carrier has no gun", () => {
    // Sw Pvrbv 551 has an ATGM and nothing else in the source, and the
    // generator must not helpfully give it a main gun or a coax.
    const carrier = CATALOGUE_PLATFORMS["tankmodels/sw_pvrbv_551"];
    expect(carrier.capabilities.map((c) => c.kind)).toEqual(["atm"]);
  });

  it("records what the catalogue actually holds", () => {
    // 819 names, 72 behaviours, all of them heavy armour. Worth keeping in
    // front of anybody who reads "1,239 platforms" and imagines variety.
    expect(CATALOGUE_COVERAGE.playable).toBe(819);
    expect(CATALOGUE_COVERAGE.distinctStatlines).toBe(72);
    expect(CATALOGUE_COVERAGE.playableSubclasses).toEqual(["heavyVehicle"]);
  });
});

describe("the declared overlay", () => {
  it("survives the generated catalogue", () => {
    // A regeneration overwrites CATALOGUE_PLATFORMS wholesale. If the spread
    // order were ever reversed, infantry and the mortar — the only non-armour
    // in the game — would vanish without a test failing anywhere else.
    for (const id of Object.keys(PINNED_PLATFORMS)) {
      expect(PLATFORM_SNAPSHOT[id]).toBe(PINNED_PLATFORMS[id]);
    }
  });

  it("says so on every row whose figures are invented", () => {
    // The overlay holds two kinds: platforms a person read out of the source
    // before the generator existed, whose figures are the source's, and
    // platforms the source does not model at all. Only the second kind may
    // carry an id the catalogue has never heard of, and it must say why.
    for (const [id, platform] of Object.entries(PINNED_PLATFORMS)) {
      if (id.startsWith("declared/")) {
        expect(platform.valuesDeclared, platform.displayName).toBeTruthy();
      }
    }
  });

  it("is the only place non-armour comes from, and every row says where", () => {
    // ⚠ THIS TEST USED TO READ "every non-armour platform is INVENTED", and
    // that was true for as long as infantry had no source at all: the
    // simulator catalogue names 76 foot platforms and models none of them, so
    // "not a vehicle" and "figures are ours" were the same statement.
    //
    // The curated section profile broke that equivalence — dismounted troops
    // with published tables of organisation are non-armour AND sourced. So
    // the rule is no longer "must be declared" but "must say which", and a
    // row that claims neither is the thing worth catching: a figure with no
    // provenance at all.
    const nonArmour = Object.values(PLATFORM_SNAPSHOT).filter(
      (platform) => platform.targetClass !== "armoured_vehicle",
    );
    expect(nonArmour.length).toBeGreaterThan(0);
    for (const platform of nonArmour) {
      expect(
        Boolean(platform.valuesDeclared) || Boolean(platform.sourcedFrom),
      ).toBe(true);
    }
  });
});

describe("fielding any platform from the catalogue", () => {
  it("builds a symmetric mirror match from an asset id", () => {
    // This is what lets the Play screen offer 819 platforms without anybody
    // hand-writing 819 force lists.
    const list = symmetricListFrom("tankmodels/il_merkava_mk_3b");
    const blue = list.elements.filter((element) => element.side === "blue");
    const red = list.elements.filter((element) => element.side === "red");

    expect(blue.length).toBe(red.length);
    expect(blue.every((element) => element.platform === "tankmodels/il_merkava_mk_3b")).toBe(true);
    expect(red.every((element) => element.platform === "tankmodels/il_merkava_mk_3b")).toBe(true);
  });

  it("plays a real game, not just a well-formed object", () => {
    // The point of the mirror is that both sides are identical, so it must
    // actually build into a scenario the engine accepts.
    const list = symmetricListFrom("tankmodels/us_m1a1_abrams");
    const state = scenarioFactory(list, HOUSE_V1)();
    const elements = Object.values(state.forceElements);

    expect(elements.length).toBeGreaterThan(0);
    for (const element of elements) {
      expect(element.combatStrength, element.label).toBeGreaterThan(0);
      expect(element.capabilities.length, element.label).toBeGreaterThan(0);
    }
  });

  it("carries the declared-figures warning through to the scenario", () => {
    // Fielding infantry should not quietly launder the fact that nobody
    // measured it.
    const list = symmetricListFrom("infantry/rifle_infantry");
    expect(list.caveats.some((caveat) => caveat.includes("figures are ours"))).toBe(true);
  });

  it("refuses an asset id the game does not have", () => {
    // A typo should be loud. The alternative is a scenario of nothing.
    expect(() => symmetricListFrom("tankmodels/not_a_real_tank")).toThrow();
  });
});

describe("force lists cannot name a platform that will not fight", () => {
  it("every platform in every force list exists", () => {
    // Without this a typo becomes a unit with no capabilities that silently
    // loses every engagement it is in, and the sweep reports it as a result.
    for (const list of Object.values(ALL_FORCE_LISTS)) {
      for (const element of list.elements) {
        expect(PLATFORM_SNAPSHOT[element.platform], `${list.id}: ${element.platform}`).toBeDefined();
      }
    }
  });

  it("and can actually fight", () => {
    for (const list of Object.values(ALL_FORCE_LISTS)) {
      for (const element of list.elements) {
        const platform = PLATFORM_SNAPSHOT[element.platform];
        expect(playabilityOf(platform).playable, `${list.id}: ${element.platform}`).toBe(true);
      }
    }
  });
});
