// ── bgws/data/curatedAssets.test.ts ───────────────────────────────────────
//
// The curated catalogue's row mapping and its penetration curve.
//
// The curve is the part worth testing hardest, because its SHAPE is the claim
// the chart makes: a shaped charge must plot flat and a sabot round must plot
// downward. If those two ever came out the same, the chart would be lying
// about the single most important distinction in the data.

import { describe, expect, it } from "vitest";

import type { Table } from "../../../shared/lib/csv";
import {
  curveFalloff,
  hasCurve,
  parseCuratedMunitions,
  parseCuratedPlatforms,
  penetrationCurve,
  buildCuratedCapabilityQuery,
  type CuratedMunition,
} from "./curatedAssets";

/**
 * A result set in the shape the CSV parser actually produces.
 *
 * Every field is a STRING, because that is what comes back over the SQL API
 * and it is the mapping's job to coerce. Feeding real numbers here would test
 * a code path production never takes.
 */
function table(columns: string[], rows: (string | null)[][]): Table {
  return { columns, rows: rows.map((row) => row.map((cell) => cell ?? "")) };
}

function munition(overrides: Partial<CuratedMunition>): CuratedMunition {
  return {
    munitionId: "m_test",
    weaponId: "w_test",
    name: "Test round",
    kind: "apfsds",
    isKinetic: true,
    isShapedCharge: false,
    topAttack: false,
    defeatsEraKe: false,
    penMm0m: null,
    penMm1000m: null,
    penMm2000m: null,
    penMm3000m: null,
    lethalRadiusM: null,
    heFillKg: null,
    sourceConfidence: null,
    sourceRef: null,
    ...overrides,
  };
}

describe("the penetration curve", () => {
  it("plots a kinetic round as a descending line", () => {
    // L27A1 CHARM 3, as published.
    const points = penetrationCurve(
      munition({ penMm0m: 676, penMm1000m: 657, penMm2000m: 620, penMm3000m: 583 }),
    );
    expect(points).toHaveLength(4);
    expect(points.map((p) => p.rangeM)).toEqual([0, 1000, 2000, 3000]);
    // Strictly descending: each point below the one before it.
    for (let i = 1; i < points.length; i += 1) {
      expect(points[i].penMm).toBeLessThan(points[i - 1].penMm);
    }
  });

  it("plots a shaped charge as a flat line", () => {
    // ⚠ THE DISTINCTION THE CHART EXISTS TO SHOW. The jet is formed on
    // impact, so a Kornet defeats the same armour at 3 km as at zero.
    const points = penetrationCurve(
      munition({
        kind: "heat_tandem",
        isKinetic: false,
        isShapedCharge: true,
        penMm0m: 1200,
        penMm1000m: 1200,
        penMm2000m: 1200,
        penMm3000m: 1200,
      }),
    );
    expect(points.map((p) => p.penMm)).toEqual([1200, 1200, 1200, 1200]);
    // Zero falloff is the assertion that matters: the line is flat, not
    // merely shallow.
    expect(
      curveFalloff(
        munition({ isShapedCharge: true, penMm0m: 1200, penMm3000m: 1200 }),
      ),
    ).toBe(0);
  });

  it("measures the falloff a reader would otherwise have to subtract", () => {
    const sabot = munition({ penMm0m: 676, penMm3000m: 583 });
    const falloff = curveFalloff(sabot);
    expect(falloff).not.toBeNull();
    // Roughly a seventh lost over three kilometres.
    expect(falloff!).toBeCloseTo((676 - 583) / 676, 5);
  });

  it("plots nothing for a round with no penetration", () => {
    // ⚠ NULL IS NOT ZERO. A high-explosive shell has no anti-armour
    // penetration to record, which is a different statement from having run
    // out of it, and a zero on the chart would say the second thing.
    const he = munition({ kind: "he", isKinetic: false });
    expect(penetrationCurve(he)).toEqual([]);
    expect(hasCurve(he)).toBe(false);
    expect(curveFalloff(he)).toBeNull();
  });

  it("refuses to draw a curve through one point", () => {
    // One number is not a trend, and a single dot invites the reader to
    // infer one.
    expect(hasCurve(munition({ penMm1000m: 500 }))).toBe(false);
    expect(hasCurve(munition({ penMm1000m: 500, penMm2000m: 470 }))).toBe(true);
  });
});

describe("mapping curated rows", () => {
  it("reads the five facings into kinetic and chemical pairs", () => {
    const rows = parseCuratedPlatforms(
      table(
        [
          "asset_id",
          "display_name",
          "armour_hull_front_ke_mm",
          "armour_hull_front_ce_mm",
          "armour_side_ke_mm",
          "armour_side_ce_mm",
          "armour_roof_ke_mm",
          "armour_roof_ce_mm",
        ],
        [["var_11_default", "Challenger 2", "700", "1000", "140", "400", "30", "30"]],
      ),
    );

    expect(rows).toHaveLength(1);
    const byAspect = new Map(rows[0].facings.map((f) => [f.aspect, f]));
    // The asymmetry the old single-figure profile could not express.
    expect(byAspect.get("Hull front")).toMatchObject({ keMm: 700, ceMm: 1000 });
    expect(byAspect.get("Side")).toMatchObject({ keMm: 140, ceMm: 400 });
    expect(byAspect.get("Roof")).toMatchObject({ keMm: 30, ceMm: 30 });
    // Absent columns are null, not zero.
    expect(byAspect.get("Rear")).toMatchObject({ keMm: null, ceMm: null });
  });

  it("reads an array column as JSON text or as a comma-separated list", () => {
    const asJson = parseCuratedPlatforms(
      table(
        ["asset_id", "display_name", "bgws_capabilities"],
        [["var_1", "A", '["apers", "atk"]']],
      ),
    );
    const asCommas = parseCuratedPlatforms(
      table(["asset_id", "display_name", "bgws_capabilities"], [["var_1", "A", "apers,atk"]]),
    );
    expect(asJson[0].capabilities).toEqual(["apers", "atk"]);
    expect(asCommas[0].capabilities).toEqual(["apers", "atk"]);
  });

  it("skips rows with no id rather than inventing one", () => {
    const rows = parseCuratedPlatforms(
      table(["asset_id", "display_name"], [[null, "Nameless"], ["var_2", "Real"]]),
    );
    expect(rows.map((r) => r.assetId)).toEqual(["var_2"]);
  });

  it("keeps the Combat Strength inputs so the number can be recomputed", () => {
    const rows = parseCuratedPlatforms(
      table(
        ["asset_id", "display_name", "bgws_cs_index", "cs_input_pen_mm", "cs_input_armour_mm", "cs_input_range_m"],
        [["var_11_default", "Challenger 2", "8.6", "657", "700", "4000"]],
      ),
    );
    expect(rows[0]).toMatchObject({
      csIndex: 8.6,
      csInputPenMm: 657,
      csInputArmourMm: 700,
      csInputRangeM: 4000,
    });
  });

  it("maps a munition's flags, not just its numbers", () => {
    const rows = parseCuratedMunitions(
      table(
        ["munition_id", "name", "kind", "is_shaped_charge", "top_attack", "defeats_era_ke", "pen_mm_1000m"],
        [["m_javelin", "Javelin tandem HEAT", "heat_tandem", "true", "true", "false", "750"]],
      ),
    );
    expect(rows[0]).toMatchObject({
      isShapedCharge: true,
      topAttack: true,
      penMm1000m: 750,
    });
  });
});

describe("queries", () => {
  it("escapes a quote in an asset id rather than breaking the statement", () => {
    expect(buildCuratedCapabilityQuery("var_o'brien")).toContain("'var_o''brien'");
  });
});
