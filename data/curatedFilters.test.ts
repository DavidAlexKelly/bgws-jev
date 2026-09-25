// ── bgws/data/curatedFilters.test.ts ──────────────────────────────────────
//
// Faceted filtering over the equipment catalogue.
//
// Worth testing because a wrong predicate does not throw. It returns the
// wrong equipment, and a list of tanks looks equally plausible whether or not
// the filter meant what the reader thought. The two rules that are easy to get
// backwards, and that this file pins down:
//
//   OR within a facet, AND between facets — picking `mbt` and `ifv` WIDENS
//   the list; picking `mbt` and nation Russia NARROWS it.
//
//   Facet counts exclude that facet's own selection — otherwise every
//   unselected value reads zero and nothing can be widened to.

import { describe, expect, it } from "vitest";

import type { CuratedPlatform, CuratedSection } from "./curatedAssets";
import {
  activeCuratedCount,
  applyCurated,
  facetCounts,
  matchesCurated,
  rowsFromPlatforms,
  rowsFromSections,
  toggleCurated,
  type EquipmentRow,
} from "./curatedFilters";

describe("facet labels come from the pipeline, not from the UI", () => {
  it("shows the class in words while filtering on the id", () => {
    // ⚠ THE LABEL AND THE KEY ARE DIFFERENT THINGS. The reader sees "Attack
    // Helicopter"; the selection is still keyed on `attack_helo`, so editing
    // a label cannot invalidate a saved filter.
    const rows = rowsFromPlatforms([
      platform({ assetId: "a", subclass: "attack_helo", subclassLabel: "Attack Helicopter" }),
    ]);
    const [entry] = facetCounts(rows, {}, "subclasses");
    expect(entry.value).toBe("attack_helo");
    expect(entry.label).toBe("Attack Helicopter");
  });

  it("falls back to the id for a class the pipeline has not labelled", () => {
    // A second copy of the vocabulary in the app would drift; showing the raw
    // id is the honest fallback and says plainly that a label is missing.
    const rows = rowsFromPlatforms([
      platform({ assetId: "a", subclass: "novel_class", subclassLabel: null }),
    ]);
    const [entry] = facetCounts(rows, {}, "subclasses");
    expect(entry.label).toBe("novel_class");
  });

  it("finds an asset by the words on screen as well as by the id", () => {
    const rows = rowsFromPlatforms([
      platform({ assetId: "a", subclass: "attack_helo", subclassLabel: "Attack Helicopter" }),
    ]);
    expect(applyCurated(rows, { search: "attack helicopter" })).toHaveLength(1);
    expect(applyCurated(rows, { search: "attack_helo" })).toHaveLength(1);
  });
});

function platform(over: Partial<CuratedPlatform> & { assetId: string }): CuratedPlatform {
  return {
    displayName: over.assetId,
    domain: "land",
    playable: true,
    subclass: "mbt",
    subclassLabel: "MBT",
    nation: "United Kingdom",
    baseAssetId: null,
    isVariant: false,
    moveType: "T",
    targetClass: "armoured_vehicle",
    protectionBand: "heavy",
    csIndex: 8.6,
    capabilities: ["apers", "atk"],
    massT: null,
    speedKmh: null,
    crewSize: null,
    dismountsCarried: null,
    dismountTimeTurns: null,
    opticsClass: null,
    signatureClass: null,
    requiresPrimeMover: false,
    facings: [{ aspect: "Hull front", keMm: 700, ceMm: 1000 }],
    eraFit: null,
    apsFit: null,
    bestPenMm1000m: null,
    bestCalibreMm: null,
    weaponCount: null,
    csInputPenMm: null,
    csInputArmourMm: null,
    csInputRangeM: null,
    protectionConfidence: null,
    protectionSourceRef: null,
    mfId: null,
    imageFilename: null,
    imageMatchMethod: null,
    survivabilityBand: null,
    threatVulnerability: null,
    ...over,
  };
}

function section(over: Partial<CuratedSection> & { assetId: string }): CuratedSection {
  return {
    displayName: over.assetId,
    nation: "Russia",
    strength: 7,
    fireteams: 2,
    defaultCarrier: null,
    opticsClass: null,
    csIndex: 5.5,
    capabilities: ["apers", "atk"],
    weapons: [],
    apersMaxRangeM: null,
    atkMaxRangeM: null,
    atmMaxRangeM: null,
    aaMaxRangeM: null,
    idfMaxRangeM: null,
    atkPenMm1000m: null,
    atmPenMm1000m: null,
    sourceConfidence: null,
    sourceRef: null,
    ...over,
  };
}

const CATALOGUE: EquipmentRow[] = [
  ...rowsFromPlatforms([
    platform({ assetId: "cr2", displayName: "Challenger 2", subclass: "mbt" }),
    platform({ assetId: "t90", displayName: "T-90M", subclass: "mbt", nation: "Russia" }),
    platform({ assetId: "bmp2", displayName: "BMP-2", subclass: "ifv", nation: "Russia" }),
    platform({
      assetId: "su35",
      displayName: "Su-35",
      domain: "air",
      subclass: "cas",
      nation: "Russia",
      playable: false,
      facings: [{ aspect: "Hull front", keMm: null, ceMm: null }],
      capabilities: [],
    }),
    platform({
      assetId: "frigate",
      displayName: "Admiral Gorshkov",
      domain: "sea",
      subclass: "warship",
      nation: "Russia",
      playable: false,
      facings: [],
      capabilities: [],
    }),
  ]),
  ...rowsFromSections([section({ assetId: "sec_ru_mr_bmp", displayName: "RU Motor-Rifle" })]),
];

describe("reading a domain as a type", () => {
  it("names air and sea in the words a reader would click", () => {
    const byId = new Map(CATALOGUE.map((row) => [row.id, row]));
    expect(byId.get("cr2")?.type).toBe("vehicle");
    expect(byId.get("su35")?.type).toBe("aircraft");
    expect(byId.get("frigate")?.type).toBe("naval");
    // Infantry is not a domain in the source; it is a separate table.
    expect(byId.get("sec_ru_mr_bmp")?.type).toBe("infantry");
  });

  it("gives sections a subclass so the facet has no blank entry", () => {
    const rows = rowsFromSections([section({ assetId: "s1" })]);
    expect(rows[0].subclass).toBe("section");
  });
});

describe("combining facets", () => {
  it("widens within a facet — two subclasses is a union", () => {
    // ⚠ THE RULE THAT IS EASY TO INVERT. If this were an intersection, every
    // second click would empty the list.
    const both = applyCurated(CATALOGUE, { subclasses: ["mbt", "ifv"] });
    expect(both.map((row) => row.id).sort()).toEqual(["bmp2", "cr2", "t90"]);
  });

  it("narrows between facets — a subclass and a nation is an intersection", () => {
    const narrowed = applyCurated(CATALOGUE, {
      subclasses: ["mbt"],
      nations: ["Russia"],
    });
    expect(narrowed.map((row) => row.id)).toEqual(["t90"]);
  });

  it("treats an empty facet as no opinion, not as match-nothing", () => {
    expect(applyCurated(CATALOGUE, {}).length).toBe(CATALOGUE.length);
    expect(applyCurated(CATALOGUE, { subclasses: [] }).length).toBe(CATALOGUE.length);
  });

  it("matches any of the selected capabilities", () => {
    // "Show me everything that can engage armour" is a union.
    const armed = applyCurated(CATALOGUE, { capabilities: ["atk"] });
    expect(armed.map((row) => row.id).sort()).toEqual(["bmp2", "cr2", "sec_ru_mr_bmp", "t90"]);
  });
});

describe("the two flags that answer real questions", () => {
  it("playableOnly hides what the game cannot field", () => {
    const playable = applyCurated(CATALOGUE, { playableOnly: true });
    expect(playable.some((row) => row.type === "aircraft")).toBe(false);
    expect(playable.some((row) => row.type === "naval")).toBe(false);
    expect(playable.some((row) => row.type === "infantry")).toBe(true);
  });

  it("armouredOnly hides what has no armour model at all", () => {
    // Aircraft, warships and sections have no protection rows, which is
    // different from having thin armour.
    const armoured = applyCurated(CATALOGUE, { armouredOnly: true });
    expect(armoured.map((row) => row.id).sort()).toEqual(["bmp2", "cr2", "t90"]);
  });
});

describe("search", () => {
  it("looks in the name, the class and the nation", () => {
    expect(applyCurated(CATALOGUE, { search: "challenger" }).map((r) => r.id)).toEqual(["cr2"]);
    expect(applyCurated(CATALOGUE, { search: "warship" }).map((r) => r.id)).toEqual(["frigate"]);
    // Five: T-90M, BMP-2, the Su-35, the frigate and the motor-rifle section.
    expect(applyCurated(CATALOGUE, { search: "russia" }).length).toBe(5);
  });

  it("ignores case and surrounding space", () => {
    expect(matchesCurated(CATALOGUE[0], { search: "  CHALLENGER " })).toBe(true);
  });
});

describe("facet counts", () => {
  it("counts a facet against the OTHER facets, not against itself", () => {
    // ⚠ THE RULE THAT MAKES FACETS USABLE. With `mbt` selected, `ifv` must
    // still show its count, or there is no way to widen the selection.
    const counts = facetCounts(CATALOGUE, { subclasses: ["mbt"] }, "subclasses");
    const byValue = new Map(counts.map((entry) => [entry.value, entry]));
    expect(byValue.get("mbt")).toMatchObject({ count: 2, selected: true });
    expect(byValue.get("ifv")).toMatchObject({ count: 1, selected: false });
  });

  it("does narrow a facet by the other facets' selections", () => {
    const counts = facetCounts(CATALOGUE, { nations: ["Russia"] }, "subclasses");
    const byValue = new Map(counts.map((entry) => [entry.value, entry.count]));
    expect(byValue.get("mbt")).toBe(1); // T-90M only
    expect(byValue.has("section")).toBe(true); // the RU section
  });

  it("keeps a selected value visible even when nothing matches it", () => {
    // Otherwise a selection vanishes from the panel it was made in, and the
    // reader cannot clear it.
    const counts = facetCounts(
      CATALOGUE,
      { nations: ["Russia"], subclasses: ["mbt", "nonexistent"] },
      "subclasses",
    );
    const values = counts.map((entry) => entry.value);
    expect(values).toContain("nonexistent");
    expect(counts.find((entry) => entry.value === "nonexistent")?.count).toBe(0);
  });
});

describe("toggling", () => {
  it("adds then removes, and drops the group when it empties", () => {
    const on = toggleCurated({}, "subclasses", "mbt");
    expect(on.subclasses).toEqual(["mbt"]);
    const off = toggleCurated(on, "subclasses", "mbt");
    // Undefined rather than [], so "no opinion" has one representation.
    expect(off.subclasses).toBeUndefined();
  });

  it("counts what is active, so the clear affordance can be honest", () => {
    expect(activeCuratedCount({})).toBe(0);
    expect(
      activeCuratedCount({
        types: ["vehicle"],
        subclasses: ["mbt", "ifv"],
        playableOnly: true,
        search: "  ",
      }),
    ).toBe(4);
  });
});
