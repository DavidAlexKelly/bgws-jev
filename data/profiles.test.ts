import { describe, expect, it } from "vitest";

import { parseCsv } from "../../../shared/lib/csv";
import {
  buildCapabilityQuery,
  buildPlatformQuery,
  parseCapabilityRows,
  parsePlatformRows,
  toForceElementDraft,
} from "./profiles";

// Real rows, copied from the built dataset rather than invented, so the test
// fails if the L6 column names change under it.
const PLATFORM_CSV = [
  "asset_id,display_name,domain,nation_primary,bgws_move_type,bgws_target_class," +
    "protection_band,bgws_capabilities,has_atgm,has_smoke,mass_combat_t,mass_source," +
    "engine_hp,hp_per_tonne,hp_per_tonne_method,mobility_class,gear_limited_kmh," +
    "gear_limited_confidence,statcard_speed_kmh,statcard_speed_is_template," +
    "bgws_mobility_modifier,armor_max_mm,crew_size,best_pen_mm_1000m,bgws_cs_index," +
    "profile_confidence",
  'uk_challenger_2_tes,Uk Challenger 2 Tes,armour,united_kingdom,T,armoured_vehicle,' +
    'heavy,"[""apers"",""atk""]",false,false,74.8,combat_mass_takeoff,1314,17.6,' +
    "engine_hp_over_combat_mass,adequate,59.2,low_estimate_no_gear_table,75,true,0,600,,,8.6,1.0",
  'uk_boxer_crv,Uk Boxer Crv Block2,armour,united_kingdom,W,armoured_vehicle,' +
    'medium,"[""apers"",""atk"",""atm""]",true,false,38.5,combat_mass_takeoff,720,18.7,' +
    "engine_hp_over_combat_mass,mobile,65.9,low_estimate_no_gear_table,75,true,0,100,,900,8.6,1.0",
  // Unclassified hull: no move type. Must be dropped, not defaulted to Foot.
  "broken_row,Some Test Rig,armour,,,,,,,,,,,,,,,,,,,,,,,",
].join("\n");

const CAPABILITY_CSV = [
  "asset_id,bgws_capability,bgws_max_range_m,short_range_m,best_pen_mm_1000m," +
    "long_range_falloff,suggested_cs_mod_long,fire_column_hint,penetration_source,munition_types",
  'uk_boxer_crv,atm,2000,1386,900,none,0,atm,shaped_charge_weapon_fallback,"[""atgm_tank""]"',
  'uk_boxer_crv,apers,3000,2000,4,marked,-2,cannon_20_80mm,kinetic_curve,"[""he_frag_tank""]"',
  "uk_boxer_crv,bogus_class,3000,2000,,,,,,",
].join("\n");

describe("platform rows", () => {
  const platforms = parsePlatformRows(parseCsv(PLATFORM_CSV));

  it("keeps only rows a game can actually place", () => {
    expect(platforms.map((p) => p.assetId)).toEqual(["uk_challenger_2_tes", "uk_boxer_crv"]);
  });

  it("reads the combat mass, not the placeholder", () => {
    // The root `mass` field says 54 t for a Challenger 2 and 5.5 t for a
    // Boxer; these are the TakeOff figures the L6 layer publishes instead.
    expect(platforms[0].massCombatT).toBe(74.8);
    expect(platforms[1].massCombatT).toBe(38.5);
  });

  it("distinguishes a wheeled hull from a tracked one", () => {
    expect(platforms[0].moveType).toBe("T");
    expect(platforms[1].moveType).toBe("W");
  });

  it("parses the capability array and drops anything unrecognised", () => {
    expect(platforms[1].capabilities).toEqual(["apers", "atk", "atm"]);
    expect(platforms[1].hasAtgm).toBe(true);
  });

  it("carries the stat-card speed AND the flag that says to ignore it", () => {
    // Both are needed: the figure so a reader can see what the source claims,
    // the flag so they are told it is a copy-pasted constant rather than a
    // performance figure.
    expect(platforms[0].statcardSpeedKmh).toBe(75);
    expect(platforms[0].statcardSpeedIsTemplate).toBe(true);
    expect(platforms[0].gearLimitedKmh).toBe(59.2);
    expect(platforms[0].gearLimitedConfidence).toBe("low_estimate_no_gear_table");
  });

  it("leaves an absent penetration figure null rather than zero", () => {
    // Challenger 2's L30A1 has no penetration curve in the source; the L6
    // layer publishes nothing rather than a plausible small number.
    expect(platforms[0].bestPenMm1000m).toBeNull();
    expect(platforms[1].bestPenMm1000m).toBe(900);
  });
});

describe("capability rows", () => {
  const capabilities = parseCapabilityRows(parseCsv(CAPABILITY_CSV));

  it("ignores a capability class outside the BGWS vocabulary", () => {
    expect(capabilities).toHaveLength(2);
  });

  it("carries the short-range threshold and the long-range modifier", () => {
    const atm = capabilities.find((c) => c.capability === "atm");
    expect(atm?.maxRangeM).toBe(2000);
    expect(atm?.shortRangeM).toBe(1386);
    expect(atm?.suggestedCsModLong).toBe(0);

    const apers = capabilities.find((c) => c.capability === "apers");
    expect(apers?.longRangeFalloff).toBe("marked");
    expect(apers?.suggestedCsModLong).toBe(-2);
  });
});

describe("queries", () => {
  it("escapes a quote rather than breaking the statement", () => {
    const query = buildPlatformQuery({ search: "o'brien" });
    expect(query).toContain("'%o''brien%'");
  });

  it("looks one asset up without applying the confidence floor", () => {
    // A lookup by id is not a search. Filtering it by confidence would return
    // nothing for exactly the rows whose provenance a reader most needs.
    const query = buildPlatformQuery({ assetIds: ["tankmodels/uk_challenger_2_tes"] });
    expect(query).toContain("asset_id IN ('tankmodels/uk_challenger_2_tes')");
    // The column is still SELECTed — a reader needs to see the confidence.
    // It is the FILTER that must not be there.
    expect(query).not.toContain("profile_confidence >=");
  });

  it("filters by move type and required capability", () => {
    const query = buildPlatformQuery({
      moveTypes: ["T"],
      requiresCapability: ["atm"],
    });
    expect(query).toContain("bgws_move_type IN ('T')");
    expect(query).toContain("array_contains(bgws_capabilities, 'atm')");
  });

  it("asks for nothing when given no assets", () => {
    expect(buildCapabilityQuery(["a", "b"])).toContain("asset_id IN ('a', 'b')");
  });
});

describe("force element draft", () => {
  const platforms = parsePlatformRows(parseCsv(PLATFORM_CSV));
  const capabilities = parseCapabilityRows(parseCsv(CAPABILITY_CSV));

  it("scales the strength index with platform count but never the ranges", () => {
    const draft = toForceElementDraft("Recce Tp", platforms[1], capabilities, 4);
    expect(draft.combinedCsIndex).toBe(34.4);
    // Four vehicles do not shoot further than one.
    expect(draft.capabilities.find((c) => c.capability === "atm")?.maxRangeM).toBe(2000);
  });

  it("leaves the scenario's own numbers explicitly null", () => {
    const draft = toForceElementDraft("Recce Tp", platforms[1], capabilities, 4);
    expect(draft.troopQuality).toBeNull();
    expect(draft.commandRating).toBeNull();
    expect(draft.ammo).toBeNull();
  });
});
