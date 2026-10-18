import { describe, expect, it } from "vitest";

import { parseCsv } from "../../../shared/lib/csv";
import {
  activeFilterCount,
  buildAssetQuery,
  buildFacetCountsQuery,
  buildMatchCountQuery,
  canonicalExpr,
  cycleExists,
  cycleFlag,
  filterPredicates,
  groupFacetCounts,
  NULL_FACET,
  parseFacetCounts,
  setRange,
  toggleFacetValue,
  type AssetFilters,
} from "./assetFilters";
import { UNIT_IMAGE_INDEX_RID } from "./unitImageIndex";

describe("search", () => {
  it("looks in the asset id as well as the display name", () => {
    const query = buildAssetQuery({ search: "tankmodels" });
    expect(query).toContain("LOWER(display_name) LIKE '%tankmodels%'");
    expect(query).toContain("LOWER(asset_id) LIKE '%tankmodels%'");
  });

  it("escapes a quote rather than breaking the statement", () => {
    expect(buildAssetQuery({ search: "o'brien" })).toContain("'%o''brien%'");
  });

  it("ignores whitespace-only input", () => {
    // Otherwise a stray space becomes LIKE '%%', which matches everything and
    // looks identical to no filter while costing a full scan.
    expect(filterPredicates({ search: "   " })).toEqual(["asset_id IS NOT NULL"]);
  });
});

describe("facets", () => {
  it("filters on a single value", () => {
    expect(filterPredicates({ facets: { domain: ["armour"] } })).toContain(
      "COALESCE(domain, '(none)') IN ('armour')",
    );
  });

  it("treats several values in one facet as OR", () => {
    const predicates = filterPredicates({ facets: { domain: ["armour", "naval"] } });
    expect(predicates.some((p) => p.includes("IN ('armour', 'naval')"))).toBe(true);
  });

  it("combines different facets as AND", () => {
    const predicates = filterPredicates({
      facets: { domain: ["armour"], subclass: ["heavyVehicle"] },
    });
    expect(predicates).toHaveLength(3); // the id guard plus one per facet
  });

  it("ignores an empty selection", () => {
    expect(filterPredicates({ facets: { domain: [] } })).toEqual(["asset_id IS NOT NULL"]);
  });

  it("matches nulls through the sentinel rather than dropping them", () => {
    // 387 assets have no armour class and 261 no nation. A chip for "(none)"
    // has to reach them, and `col IN ('(none)')` never would — hence the
    // COALESCE in the expression on both sides.
    const predicate = filterPredicates({ facets: { nation: [NULL_FACET] } })[1];
    expect(predicate).toContain("COALESCE(");
    expect(predicate).toContain("'(none)'");
  });
});

describe("nation aliasing", () => {
  it("folds britain into united_kingdom and usa into united_states", () => {
    const expression = canonicalExpr("nation");
    expect(expression).toContain("'britain'");
    expect(expression).toContain("THEN 'united_kingdom'");
    expect(expression).toContain("'usa'");
    expect(expression).toContain("THEN 'united_states'");
  });

  it("selecting united_kingdom therefore reaches the britain rows", () => {
    const predicate = filterPredicates({ facets: { nation: ["united_kingdom"] } })[1];
    expect(predicate).toContain("'britain'");
    expect(predicate).toContain("IN ('united_kingdom')");
  });

  it("uses ONE expression for both the filter and the count", () => {
    // If these drift the chips stop agreeing with the results, and only for
    // the rows nobody checks. Same string or nothing.
    const expression = canonicalExpr("nation");
    expect(buildFacetCountsQuery()).toContain(`GROUP BY ${expression}`);
    expect(filterPredicates({ facets: { nation: ["germany"] } })[1]).toContain(expression);
  });

  it("leaves a facet with no aliases as a plain column", () => {
    expect(canonicalExpr("domain")).toBe("COALESCE(domain, '(none)')");
    expect(canonicalExpr("domain")).not.toContain("CASE");
  });
});

describe("flags", () => {
  it("requires true", () => {
    expect(filterPredicates({ flags: { armed: true } })).toContain(
      "COALESCE(is_armed, false) = true",
    );
  });

  it("requires false", () => {
    expect(filterPredicates({ flags: { armed: false } })).toContain(
      "COALESCE(is_armed, false) = false",
    );
  });

  it("COALESCEs rather than comparing directly", () => {
    // `is_armed = false` drops null rows from BOTH sides of the filter, so
    // the two halves would not sum to the catalogue. That is the one answer
    // that is certainly wrong.
    expect(filterPredicates({ flags: { mobile: false } })[1]).toContain("COALESCE(");
  });

  it("leaves the column alone when unset", () => {
    expect(filterPredicates({ flags: {} })).toEqual(["asset_id IS NOT NULL"]);
  });
});

describe("presence filters", () => {
  it("asks the image index for hasImage", () => {
    const predicate = filterPredicates({ exists: { hasImage: true } })[1];
    expect(predicate).toContain("asset_id IN (SELECT asset_id FROM");
    expect(predicate).toContain(UNIT_IMAGE_INDEX_RID);
  });

  it("guards NOT IN against nulls in the subquery", () => {
    // A single null in a NOT IN list makes the whole predicate unknown and
    // returns nothing at all — a filter that silently empties the list.
    const predicate = filterPredicates({ exists: { hasImage: false } })[1];
    expect(predicate).toContain("NOT IN");
    expect(predicate).toContain("WHERE asset_id IS NOT NULL");
  });

  it("asks the platform profile for inWargame", () => {
    const predicate = filterPredicates({ exists: { inWargame: true } })[1];
    expect(predicate).toContain("IN (SELECT asset_id FROM");
  });
});

describe("ranges", () => {
  it("applies a minimum", () => {
    expect(filterPredicates({ ranges: { massT: { min: 40 } } })).toContain("mass_t >= 40");
  });

  it("applies both bounds", () => {
    const predicates = filterPredicates({ ranges: { massT: { min: 40, max: 70 } } });
    expect(predicates).toContain("mass_t >= 40");
    expect(predicates).toContain("mass_t <= 70");
  });

  it("ignores a bound that is not a finite number", () => {
    // An emptied input arrives as NaN via Number(""), and `col >= NaN` is a
    // predicate that matches nothing.
    expect(filterPredicates({ ranges: { massT: { min: Number.NaN } } })).toEqual([
      "asset_id IS NOT NULL",
    ]);
  });

  it("accepts zero as a real bound", () => {
    // Falsy, and easy to drop with a truthiness check.
    expect(filterPredicates({ ranges: { armourMaxMm: { min: 0 } } })).toContain(
      "armor_max_mm >= 0",
    );
  });
});

describe("sorting", () => {
  it("defaults to name", () => {
    expect(buildAssetQuery({})).toContain("ORDER BY display_name");
  });

  it("puts nulls last on a descending sort", () => {
    // An asset with no mass is not the heaviest thing in the catalogue.
    expect(buildAssetQuery({ sort: "massDesc" })).toContain("mass_t DESC NULLS LAST");
  });
});

describe("the match count query", () => {
  it("uses the same predicates as the list but no limit", () => {
    const filters: AssetFilters = { search: "tank", facets: { domain: ["armour"] } };
    const count = buildMatchCountQuery(filters);
    for (const predicate of filterPredicates(filters)) {
      expect(count).toContain(predicate);
    }
    expect(count).not.toContain("LIMIT");
  });
});

describe("facet counts", () => {
  it("names the null bucket rather than dropping it", () => {
    const table = parseCsv(
      ["facet,value,n", "domain,armour,1046", `nation,${NULL_FACET},261`].join("\n"),
    );
    expect(parseFacetCounts(table)).toEqual([
      { facet: "domain", value: "armour", count: 1046 },
      { facet: "nation", value: NULL_FACET, count: 261 },
    ]);
  });

  it("drops a facet name it does not recognise", () => {
    // Means the query and this parser have drifted; an untitled chip group is
    // worse than a missing one.
    const table = parseCsv(["facet,value,n", "invented,x,5"].join("\n"));
    expect(parseFacetCounts(table)).toEqual([]);
  });

  it("groups every known facet, including the empty ones", () => {
    const grouped = groupFacetCounts([{ facet: "domain", value: "armour", count: 1 }]);
    expect(grouped.domain).toHaveLength(1);
    expect(grouped.nation).toEqual([]);
  });
});

describe("filter state helpers", () => {
  it("toggles a facet value on and back off", () => {
    const once = toggleFacetValue({}, "domain", "armour");
    expect(once.facets?.domain).toEqual(["armour"]);
    const twice = toggleFacetValue(once, "domain", "armour");
    expect(twice.facets?.domain).toBeUndefined();
  });

  it("removes the facet key entirely when the last value goes", () => {
    // Left as an empty array it would still count as an active filter.
    const cleared = toggleFacetValue(toggleFacetValue({}, "domain", "armour"), "domain", "armour");
    expect(activeFilterCount(cleared)).toBe(0);
  });

  it("cycles a flag through true, false and unset", () => {
    const a = cycleFlag({}, "armed");
    expect(a.flags?.armed).toBe(true);
    const b = cycleFlag(a, "armed");
    expect(b.flags?.armed).toBe(false);
    const c = cycleFlag(b, "armed");
    expect(c.flags?.armed).toBeUndefined();
    expect(activeFilterCount(c)).toBe(0);
  });

  it("cycles a presence filter the same way", () => {
    const a = cycleExists({}, "hasImage");
    expect(a.exists?.hasImage).toBe(true);
    expect(cycleExists(cycleExists(a, "hasImage"), "hasImage").exists?.hasImage).toBeUndefined();
  });

  it("drops an emptied range", () => {
    const set = setRange({}, "massT", { min: 40 });
    expect(activeFilterCount(set)).toBe(1);
    expect(activeFilterCount(setRange(set, "massT", {}))).toBe(0);
  });

  it("counts each active filter once", () => {
    const filters: AssetFilters = {
      search: "tank",
      facets: { domain: ["armour", "naval"] },
      flags: { armed: true },
      exists: { hasImage: true },
      ranges: { massT: { min: 40 } },
    };
    // Two values in one facet is one filter, not two.
    expect(activeFilterCount(filters)).toBe(5);
  });

  it("does not count a sort as a filter", () => {
    expect(activeFilterCount({ sort: "massDesc" })).toBe(0);
  });
});

describe("everything at once", () => {
  it("produces one statement with every predicate ANDed", () => {
    const query = buildAssetQuery({
      search: "leopard",
      facets: { domain: ["armour"], nation: ["germany"] },
      flags: { armed: true },
      exists: { inWargame: true },
      ranges: { massT: { min: 40, max: 70 } },
      sort: "massDesc",
      limit: 50,
    });

    expect(query).toContain("LOWER(display_name) LIKE '%leopard%'");
    expect(query).toContain("IN ('armour')");
    expect(query).toContain("IN ('germany')");
    expect(query).toContain("COALESCE(is_armed, false) = true");
    expect(query).toContain("IN (SELECT asset_id FROM");
    expect(query).toContain("mass_t >= 40");
    expect(query).toContain("mass_t <= 70");
    expect(query).toContain("ORDER BY mass_t DESC NULLS LAST");
    expect(query).toContain("LIMIT 50");
  });
});
