// ── bgws/data/assetCards.ts ────────────────────────────────────────────────
// The asset explorer's data: [SIM] L5 sim_asset_card, every domain.
//
// Distinct from data/profiles.ts on purpose. That reads the L6 wargame
// profiles — land only, 1,239 rows, BGWS vocabulary. This reads the layer
// underneath: all 2,258 assets with their raw engineering stats, which is what
// an explorer is for.
//
// Pure: types, row mapping and query builders. The call that reaches Foundry
// is in ./assetCardsClient.

import {
  asBoolean,
  asNumber,
  asStringArray,
  asText,
  columnIndex,
  type Table,
} from "../../../shared/lib/csv";

/** [SIM] L5 sim_asset_card. */
export const ASSET_CARD_RID = "ri.foundry.main.dataset.50abec74-4779-4a17-bdd8-bbf6dab77d25";

export interface AssetCard {
  assetId: string;
  displayName: string;
  domain: string | null;
  subclass: string | null;
  nation: string | null;
  yearFrom: number | null;
  yearTo: number | null;
  isVariant: boolean;
  baseAssetId: string | null;

  massT: number | null;
  maxSpeedMps: number | null;
  /** How the speed was obtained — worth showing, because it is often a clamp. */
  speedSource: string | null;
  isMobile: boolean;
  hpTotal: number | null;
  armourMaxMm: number | null;
  armourClass: string | null;
  crewSize: number | null;

  isArmed: boolean;
  weaponCount: number | null;
  primaryWeaponRef: string | null;
  primaryWeaponClass: string | null;
  primaryCalibreMm: number | null;
  primaryRateOfFireRpm: number | null;
  primaryMuzzleVelocityMps: number | null;
  primaryExplosiveMassKg: number | null;
  munitionFamilies: string[];
  maxEngagementRangeM: number | null;
  rangeSource: string | null;
  engagesAir: boolean;
  engagesGround: boolean;
  detectDistanceM: number | null;

  completeness: number | null;
  sourcePath: string | null;
}

/**
 * The columns the explorer reads.
 *
 * Exported because ./assetFilters builds the SELECT and this module owns the
 * row shape. Keeping the list next to the parser that consumes it is the
 * point: a column added to one and not the other is a silent undefined.
 */
export const ASSET_CARD_COLUMNS = [
  "asset_id",
  "display_name",
  "domain",
  "subclass",
  "nation_primary",
  "year_from",
  "year_to",
  "is_variant",
  "base_asset_id",
  "mass_t",
  "max_speed_mps",
  "speed_source",
  "is_mobile",
  "hp_total",
  "armor_max_mm",
  "armor_class_primary",
  "crew_size",
  "is_armed",
  "weapon_count",
  "primary_weapon_ref",
  "primary_weapon_class",
  "primary_caliber_mm",
  "primary_rate_of_fire_rpm",
  "primary_muzzle_velocity_mps",
  "primary_explosive_mass_kg",
  "primary_munition_families",
  "max_engagement_range_m",
  "range_source",
  "engages_air",
  "engages_ground",
  "detect_distance_m",
  "data_completeness_score",
  "source_path",
].join(", ");

export function parseAssetCards(table: Table): AssetCard[] {
  const at = columnIndex(table);
  const cards: AssetCard[] = [];

  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const assetId = asText(get("asset_id"));
    if (!assetId) continue;

    cards.push({
      assetId,
      displayName: asText(get("display_name")) ?? assetId,
      domain: asText(get("domain")),
      subclass: asText(get("subclass")),
      nation: asText(get("nation_primary")),
      yearFrom: asNumber(get("year_from")),
      yearTo: asNumber(get("year_to")),
      isVariant: asBoolean(get("is_variant")) ?? false,
      baseAssetId: asText(get("base_asset_id")),
      massT: asNumber(get("mass_t")),
      maxSpeedMps: asNumber(get("max_speed_mps")),
      speedSource: asText(get("speed_source")),
      isMobile: asBoolean(get("is_mobile")) ?? false,
      hpTotal: asNumber(get("hp_total")),
      armourMaxMm: asNumber(get("armor_max_mm")),
      armourClass: asText(get("armor_class_primary")),
      crewSize: asNumber(get("crew_size")),
      isArmed: asBoolean(get("is_armed")) ?? false,
      weaponCount: asNumber(get("weapon_count")),
      primaryWeaponRef: asText(get("primary_weapon_ref")),
      primaryWeaponClass: asText(get("primary_weapon_class")),
      primaryCalibreMm: asNumber(get("primary_caliber_mm")),
      primaryRateOfFireRpm: asNumber(get("primary_rate_of_fire_rpm")),
      primaryMuzzleVelocityMps: asNumber(get("primary_muzzle_velocity_mps")),
      primaryExplosiveMassKg: asNumber(get("primary_explosive_mass_kg")),
      munitionFamilies: asStringArray(get("primary_munition_families")),
      maxEngagementRangeM: asNumber(get("max_engagement_range_m")),
      rangeSource: asText(get("range_source")),
      engagesAir: asBoolean(get("engages_air")) ?? false,
      engagesGround: asBoolean(get("engages_ground")) ?? false,
      detectDistanceM: asNumber(get("detect_distance_m")),
      completeness: asNumber(get("data_completeness_score")),
      sourcePath: asText(get("source_path")),
    });
  }

  return cards;
}

// Query building moved to ./assetFilters when the explorer grew from one
// domain chip to the full facet set. The filter model has to know about
// nulls, alias folding and subquery predicates, none of which belong in the
// module that maps rows.

/** Metres per second into something a reader recognises. */
export function kmh(metresPerSecond: number | null): number | null {
  if (metresPerSecond == null) return null;
  return Math.round(metresPerSecond * 3.6);
}
