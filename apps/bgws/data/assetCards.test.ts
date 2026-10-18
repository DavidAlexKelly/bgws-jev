import { describe, expect, it } from "vitest";

import { parseCsv } from "../../../shared/lib/csv";
import { ASSET_CARD_COLUMNS, kmh, parseAssetCards } from "./assetCards";
import { imageCandidates } from "./unitImagePaths";

// Real rows and real ids, copied from the built dataset.
const CSV = [
  "asset_id,display_name,domain,subclass,nation_primary,year_from,year_to,is_variant," +
    "base_asset_id,mass_t,max_speed_mps,speed_source,is_mobile,hp_total,armor_max_mm," +
    "armor_class_primary,crew_size,is_armed,weapon_count,primary_weapon_ref," +
    "primary_weapon_class,primary_caliber_mm,primary_rate_of_fire_rpm," +
    "primary_muzzle_velocity_mps,primary_explosive_mass_kg,primary_munition_families," +
    "max_engagement_range_m,range_source,engages_air,engages_ground,detect_distance_m," +
    "data_completeness_score,source_path",
  'tankmodels/germ_leopard_1a5,Germ Leopard 1a5,armour,heavyVehicle,germany,,,false,,' +
    '42.4,20.8,declared,true,1200,600,rolled_homogeneous,4,true,3,120mm_kan,' +
    'groundmodels_weapons,105,8,1478,0.9,"[""bullet""]",7000,declared,false,true,,1,x.blk',
  "humans/m24,M24,infantry,human,,,,false,,,3.7,posture,true,,,,,,,,,,,,,,,,false,false,,0.6,y.blk",
].join("\n");

describe("asset cards", () => {
  const cards = parseAssetCards(parseCsv(CSV));

  it("reads both an armoured vehicle and a soldier", () => {
    expect(cards).toHaveLength(2);
    expect(cards[0].displayName).toBe("Germ Leopard 1a5");
    expect(cards[1].domain).toBe("infantry");
  });

  it("keeps the provenance fields, because the values need them", () => {
    // speed_source says whether a figure is declared or a clamp — the reason
    // 75 km/h appears on every tank in this source.
    expect(cards[0].speedSource).toBe("declared");
    expect(cards[0].rangeSource).toBe("declared");
  });

  it("leaves absent numbers null rather than zero", () => {
    // Zero armour and unknown armour are different facts about a soldier.
    expect(cards[1].armourMaxMm).toBeNull();
    expect(cards[1].massT).toBeNull();
  });

  it("parses the munition family array", () => {
    expect(cards[0].munitionFamilies).toEqual(["bullet"]);
    expect(cards[1].munitionFamilies).toEqual([]);
  });

  it("converts speed into something a reader recognises", () => {
    expect(kmh(cards[0].maxSpeedMps)).toBe(75);
    expect(kmh(null)).toBeNull();
  });
});

// Query building and filtering moved to ./assetFilters — see
// assetFilters.test.ts. What belongs here is the pairing between the columns
// requested and the fields parsed out of them.

describe("the column list and the parser agree", () => {
  it("selects every column the parser reads", () => {
    // A column added to the type and parser but not to the SELECT is a
    // silent undefined on every row, which renders as an em dash and looks
    // exactly like absent data.
    const requested = new Set(ASSET_CARD_COLUMNS.split(", "));
    const read = parseCsv(CSV).columns;
    for (const column of read) {
      expect(requested.has(column)).toBe(true);
    }
  });

  it("reads every column it selects", () => {
    // The other direction: a column selected and never read is dead weight
    // in the query, and usually means a rename went half-done.
    const read = new Set(parseCsv(CSV).columns);
    for (const column of ASSET_CARD_COLUMNS.split(", ")) {
      expect(read.has(column)).toBe(true);
    }
  });
});

describe("image lookup", () => {
  it("tries the display name before the asset id", () => {
    // Every filename visible in the media set follows the display-name
    // convention: Germ_Leopard_1a5.jpg, Colt_9_Mm_Smg.jpg, M24.jpg.
    const candidates = imageCandidates("tankmodels/germ_leopard_1a5", "Germ Leopard 1a5");
    expect(candidates[0]).toBe("Germ_Leopard_1a5.jpg");
    expect(candidates.indexOf("Germ_Leopard_1a5.jpg")).toBeLessThan(
      candidates.indexOf("germ_leopard_1a5.jpg"),
    );
  });

  it("preserves case, because the two conventions differ only by it", () => {
    // `M24.jpg` is in the set; `m24.jpg` would be the asset id form. Lower-
    // casing the display name would find the wrong file or none at all.
    const candidates = imageCandidates("humans/m24", "M24");
    expect(candidates[0]).toBe("M24.jpg");
    expect(candidates).toContain("m24.jpg");
  });

  it("falls back to the full asset id path", () => {
    // A media set path may contain directories, and these ids are paths.
    expect(imageCandidates("tankmodels/us_m1_abrams", "Us M1 Abrams")).toContain(
      "tankmodels/us_m1_abrams.jpg",
    );
  });

  it("handles a multi-word name with several spaces", () => {
    expect(imageCandidates("humans/colt_9_mm_smg", "Colt 9 Mm Smg")[0]).toBe(
      "Colt_9_Mm_Smg.jpg",
    );
  });

  it("does not produce duplicates when the two conventions agree", () => {
    const candidates = imageCandidates("ak_74m", "ak_74m");
    expect(new Set(candidates).size).toBe(candidates.length);
  });

  it("copes with an empty display name", () => {
    const candidates = imageCandidates("humans/m24", "");
    expect(candidates[0]).toBe("m24.jpg");
  });
});
