// ── bgws/data/profiles.ts ──────────────────────────────────────────────────
// Equipment profiles for the Battlegroup Wargame System, read straight from
// the [SIM] L6 datasets over the SQL Queries API.
//
// NO ONTOLOGY OBJECTS. This is a deliberate choice, not a shortcut: the
// profiles are reference data for a scenario author, read far more often than
// written and never edited in play, so an object type would buy indexing and
// actions that nothing here wants and cost a namespace boundary that this
// app — bound to a single ontology in src/client.ts — would have to cross.
//
// WHAT THESE TABLES ARE, AND WHAT THEY ARE NOT
// --------------------------------------------
// They are the EQUIPMENT side of a Force Management Chart: Move Type,
// Capabilities (Anti-personnel / Anti-tank / Anti-tank Missile / Smoke) with
// ranges and penetration, protection, and a relative combat-strength index.
//
// They are not a Force Element. BGWS fights Platoons, Troops and Companies;
// every row here is one platform. Troop Quality, Command Rating, Morale and
// Ammo are not in them either — those are training, doctrine, runtime state
// and game balance, none of which is a property of a vehicle. Building an FE
// means combining N platforms with an ORBAT (src/shared/orbat) and a
// scenario's own numbers; `toForceElementDraft` below produces the half that
// can be derived and leaves the other half explicitly null.
//
// ⚠ SETUP: both datasets live in /Accenture/[DK] Project Space/Units, which is
// a different namespace from this application. Cross-namespace is fine, but
// each dataset must be added as a permitted resource on the OAuth app in
// Developer Console (Developer Console → app → Resources). The SQL scopes
// alone return 403 — see the note in shared/lib/sqlClient.ts.

// This module is deliberately FREE of any import that reaches src/client.ts:
// the client reads <meta> tags at module load, so anything importing it cannot
// be unit tested. The types, the row mapping and the query builders are all
// here and tested; the two functions that actually talk to Foundry live in
// profilesClient.ts beside it. Same split as shared/lib/csv vs sqlClient.
import {
  asBoolean,
  asNumber,
  asStringArray,
  asText,
  columnIndex,
  type Table,
} from "../../../shared/lib/csv";
import { sqlLiteral } from "../../../shared/lib/sql";

// ─── Datasets ──────────────────────────────────────────────────────────────

/** [SIM] L6 bgws_platform_profile — one row per land platform. */
export const PLATFORM_PROFILE_RID =
  "ri.foundry.main.dataset.eb91f557-fa16-482d-a474-bea9c97f6aa2";

/** [SIM] L6 bgws_capability_profile — one row per platform per capability. */
export const CAPABILITY_PROFILE_RID =
  "ri.foundry.main.dataset.4d28fc58-59b3-4814-b5cf-7754054f9dac";

// ─── Types ─────────────────────────────────────────────────────────────────

/** BGWS Move Type: Foot, Wheeled, Tracked. */
export type MoveType = "F" | "W" | "T";

/** Which Capability class may engage this platform (BGWS 2.1.8). */
export type TargetClass = "foot" | "soft_skin" | "armoured_vehicle";

export type CapabilityClass =
  | "apers"
  | "atk"
  | "atm"
  /**
   * Indirect fire (9.2.2). OURS, not the source's.
   *
   * No row in bgws_platform_profile carries it, and no row ever will — the
   * catalogue has no mortar at all, not even one of the empty shells it has
   * for infantry. So only a declared platform can have this capability, which
   * is the honest consequence of the source not modelling indirect fire.
   */
  | "idf"
  | "smoke"
  | "aa"
  | "air_delivered";

export interface PlatformProfile {
  assetId: string;
  displayName: string;
  domain: string | null;
  nation: string | null;
  moveType: MoveType;
  targetClass: TargetClass;
  protectionBand: string | null;
  capabilities: CapabilityClass[];
  hasAtgm: boolean;
  hasSmoke: boolean;
  /** Combat mass in tonnes (VehiclePhys.Mass.TakeOff, not the placeholder). */
  massCombatT: number | null;
  /** combat_mass_takeoff | declared_mass_placeholder | unavailable. */
  massSource: string | null;
  engineHp: number | null;
  hpPerTonne: number | null;
  /** How power-to-weight was obtained, or why it was refused. */
  hpPerTonneMethod: string | null;
  /** sluggish | adequate | mobile | agile | unknown. */
  mobilityClass: string | null;
  /** The drivetrain's ceiling. An estimate — see its confidence. */
  gearLimitedKmh: number | null;
  gearLimitedConfidence: string | null;
  /** The source's own top speed, kept only so it can be contradicted. */
  statcardSpeedKmh: number | null;
  /** True when that figure is the value ~92% of ground vehicles share. */
  statcardSpeedIsTemplate: boolean;
  /** Suggested +/-1 to Maximum Allowable Distance. Optional rule. */
  mobilityModifier: number;
  armourMaxMm: number | null;
  crewSize: number | null;
  /** Best anti-armour penetration at 1 km, mm. Null where unknown. */
  bestPenMm1000m: number | null;
  /** Relative combat-strength index, 0-10. A calibration aid, not a CS. */
  csIndex: number | null;
  /** 0-1: how much of this row is evidence rather than absence. */
  confidence: number | null;
}

export interface CapabilityProfile {
  assetId: string;
  capability: CapabilityClass;
  /** Max range capped at the BGWS 3 km line-of-sight limit. */
  maxRangeM: number | null;
  /** Under 51% of Max Range is short range (BGWS 2.1.8). */
  shortRangeM: number | null;
  penMm1000m: number | null;
  /** none | slight | marked | unknown. */
  longRangeFalloff: string | null;
  /** Suggested CS modifier at long range. Player Aid 4 supersedes this. */
  suggestedCsModLong: number | null;
  /** Which Fire Results column this round's effect resembles. */
  fireColumnHint: string | null;
  /** How the penetration figure was obtained, or why there is none. */
  penetrationSource: string | null;
  munitionTypes: string[];
}

/**
 * The derivable half of a Force Management Chart row.
 *
 * Every field BGWS needs that a platform cannot supply is present and null,
 * because a chart with a silently missing Troop Quality is a chart somebody
 * will fill in by accident.
 */
export interface ForceElementDraft {
  label: string;
  moveType: MoveType;
  targetClass: TargetClass;
  platformCount: number;
  capabilities: CapabilityProfile[];
  /** Sum of the platforms' combat-strength indices, for calibration only. */
  combinedCsIndex: number | null;
  /** From the scenario's SSI. Never derivable from equipment. */
  troopQuality: null;
  /** HQs only, from the ORBAT. */
  commandRating: null;
  /** Game balance, set per scenario. */
  ammo: null;
  /** The evidence behind the derived half. */
  confidence: number | null;
}

// ─── Row mapping (pure — tested without a client) ──────────────────────────

function isMoveType(value: string | null): value is MoveType {
  return value === "F" || value === "W" || value === "T";
}

function isTargetClass(value: string | null): value is TargetClass {
  return value === "foot" || value === "soft_skin" || value === "armoured_vehicle";
}

const CAPABILITY_CLASSES: readonly string[] = [
  "apers",
  "atk",
  "atm",
  "smoke",
  "aa",
  "air_delivered",
];

function asCapabilities(values: string[]): CapabilityClass[] {
  return values.filter((v): v is CapabilityClass => CAPABILITY_CLASSES.includes(v));
}

export function parsePlatformRows(table: Table): PlatformProfile[] {
  const at = columnIndex(table);
  const out: PlatformProfile[] = [];

  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const moveType = asText(get("bgws_move_type"));
    const targetClass = asText(get("bgws_target_class"));

    // A row without these is not a platform this game can place. Dropped
    // rather than defaulted: guessing "Foot" for an unclassified hull would
    // put it on the map with infantry's movement allowance.
    if (!isMoveType(moveType) || !isTargetClass(targetClass)) continue;

    out.push({
      assetId: asText(get("asset_id")) ?? "",
      displayName: asText(get("display_name")) ?? asText(get("asset_id")) ?? "",
      domain: asText(get("domain")),
      nation: asText(get("nation_primary")),
      moveType,
      targetClass,
      protectionBand: asText(get("protection_band")),
      capabilities: asCapabilities(asStringArray(get("bgws_capabilities"))),
      hasAtgm: asBoolean(get("has_atgm")) ?? false,
      hasSmoke: asBoolean(get("has_smoke")) ?? false,
      massCombatT: asNumber(get("mass_combat_t")),
      massSource: asText(get("mass_source")),
      engineHp: asNumber(get("engine_hp")),
      hpPerTonne: asNumber(get("hp_per_tonne")),
      hpPerTonneMethod: asText(get("hp_per_tonne_method")),
      mobilityClass: asText(get("mobility_class")),
      gearLimitedKmh: asNumber(get("gear_limited_kmh")),
      gearLimitedConfidence: asText(get("gear_limited_confidence")),
      statcardSpeedKmh: asNumber(get("statcard_speed_kmh")),
      statcardSpeedIsTemplate: asBoolean(get("statcard_speed_is_template")) ?? false,
      mobilityModifier: asNumber(get("bgws_mobility_modifier")) ?? 0,
      armourMaxMm: asNumber(get("armor_max_mm")),
      crewSize: asNumber(get("crew_size")),
      bestPenMm1000m: asNumber(get("best_pen_mm_1000m")),
      csIndex: asNumber(get("bgws_cs_index")),
      confidence: asNumber(get("profile_confidence")),
    });
  }

  return out;
}

export function parseCapabilityRows(table: Table): CapabilityProfile[] {
  const at = columnIndex(table);
  const out: CapabilityProfile[] = [];

  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const capability = asText(get("bgws_capability"));
    if (!capability || !CAPABILITY_CLASSES.includes(capability)) continue;

    out.push({
      assetId: asText(get("asset_id")) ?? "",
      capability: capability as CapabilityClass,
      maxRangeM: asNumber(get("bgws_max_range_m")),
      shortRangeM: asNumber(get("short_range_m")),
      penMm1000m: asNumber(get("best_pen_mm_1000m")),
      longRangeFalloff: asText(get("long_range_falloff")),
      suggestedCsModLong: asNumber(get("suggested_cs_mod_long")),
      fireColumnHint: asText(get("fire_column_hint")),
      penetrationSource: asText(get("penetration_source")),
      munitionTypes: asStringArray(get("munition_types")),
    });
  }

  return out;
}

// ─── Queries ───────────────────────────────────────────────────────────────

const PLATFORM_COLUMNS = [
  "asset_id",
  "display_name",
  "domain",
  "nation_primary",
  "bgws_move_type",
  "bgws_target_class",
  "protection_band",
  "bgws_capabilities",
  "has_atgm",
  "has_smoke",
  "mass_combat_t",
  "mass_source",
  "engine_hp",
  "hp_per_tonne",
  "hp_per_tonne_method",
  "mobility_class",
  "gear_limited_kmh",
  "gear_limited_confidence",
  "statcard_speed_kmh",
  "statcard_speed_is_template",
  "bgws_mobility_modifier",
  "armor_max_mm",
  "crew_size",
  "best_pen_mm_1000m",
  "bgws_cs_index",
  "profile_confidence",
].join(", ");

const CAPABILITY_COLUMNS = [
  "asset_id",
  "bgws_capability",
  "bgws_max_range_m",
  "short_range_m",
  "best_pen_mm_1000m",
  "long_range_falloff",
  "suggested_cs_mod_long",
  "fire_column_hint",
  "penetration_source",
  "munition_types",
].join(", ");

export interface PlatformSearchOptions {
  /** Exact asset ids. Used to pull the derived profile for one asset. */
  assetIds?: string[];
  /** Substring match on display_name, case-insensitive. */
  search?: string;
  /** Restrict to these Move Types. */
  moveTypes?: MoveType[];
  /** Only platforms carrying at least one of these capabilities. */
  requiresCapability?: CapabilityClass[];
  /** Drop rows whose derivation rests on very little. Defaults to 0.4. */
  minConfidence?: number;
  limit?: number;
}

export function buildPlatformQuery(options: PlatformSearchOptions = {}): string {
  const where: string[] = [];

  // An explicit id list is a lookup, not a search: it must not be filtered by
  // confidence, or asking for one asset's profile would silently return
  // nothing for the very rows whose provenance a reader most needs to see.
  if (options.assetIds?.length) {
    where.push(`asset_id IN (${options.assetIds.map(sqlLiteral).join(", ")})`);
  } else {
    where.push(`profile_confidence >= ${options.minConfidence ?? 0.4}`);
  }

  if (options.search) {
    where.push(`LOWER(display_name) LIKE ${sqlLiteral(`%${options.search.toLowerCase()}%`)}`);
  }
  if (options.moveTypes?.length) {
    const list = options.moveTypes.map(sqlLiteral).join(", ");
    where.push(`bgws_move_type IN (${list})`);
  }
  for (const capability of options.requiresCapability ?? []) {
    where.push(`array_contains(bgws_capabilities, ${sqlLiteral(capability)})`);
  }

  return [
    `SELECT ${PLATFORM_COLUMNS}`,
    `FROM \`${PLATFORM_PROFILE_RID}\``,
    `WHERE ${where.join(" AND ")}`,
    "ORDER BY bgws_cs_index DESC, display_name",
    `LIMIT ${options.limit ?? 250}`,
  ].join("\n");
}

export function buildCapabilityQuery(assetIds: string[]): string {
  const list = assetIds.map(sqlLiteral).join(", ");
  return [
    `SELECT ${CAPABILITY_COLUMNS}`,
    `FROM \`${CAPABILITY_PROFILE_RID}\``,
    `WHERE asset_id IN (${list})`,
    "ORDER BY asset_id, bgws_capability",
  ].join("\n");
}

// ─── Composition into a Force Element ──────────────────────────────────────

/**
 * Combine N identical platforms into the derivable half of an FE row.
 *
 * The Move Type and target class come from the platform; the capabilities are
 * the platform's, unchanged — BGWS ranges are per weapon, and four tanks do
 * not shoot further than one. Only the combat-strength index scales with
 * count, and even that is a calibration aid rather than a Combat Strength.
 */
export function toForceElementDraft(
  label: string,
  platform: PlatformProfile,
  capabilities: CapabilityProfile[],
  platformCount: number
): ForceElementDraft {
  return {
    label,
    moveType: platform.moveType,
    targetClass: platform.targetClass,
    platformCount,
    capabilities,
    combinedCsIndex:
      platform.csIndex == null ? null : Math.round(platform.csIndex * platformCount * 10) / 10,
    troopQuality: null,
    commandRating: null,
    ammo: null,
    confidence: platform.confidence,
  };
}
