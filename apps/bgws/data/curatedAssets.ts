// ── bgws/data/curatedAssets.ts ─────────────────────────────────────────────
// The asset explorer's SECOND catalogue: the curated L7 profiles.
//
// WHY A SECOND CATALOGUE RATHER THAN MORE COLUMNS ON THE FIRST
// ------------------------------------------------------------
// ⚠ THE TWO CATALOGUES DO NOT SHARE IDENTIFIERS, AND PRETENDING THEY DO WOULD
// BE THE WORST THING THIS FILE COULD DO.
//
// The explorer browses [SIM] L5 sim_asset_card, whose ids look like
// `tankmodels/uk_challenger_2_bn`. The curated tables use their own, which
// look like `var_11_default`, and their `l6_asset_id` column is empty for
// every one of the 162 rows. There is no join.
//
// The tempting move is to match them by display name. Matching Military
// Factory to the simulator by name is exactly the exercise that produced 10
// matches out of 133 and a string of wrong ones — "M2 Bradley" to the M2
// Browning machine gun, a Soviet D-30 howitzer to a French Panhard armoured
// car, "Challenger 3" to a Challenger 1 — because the catalogue does not call
// vehicles by their names. Doing it again here, silently, to decorate a UI,
// would put those same wrong figures on screen with no warning at all.
//
// So the curated data is browsed AS ITSELF, in its own mode, and the explorer
// says which catalogue you are looking at. When a real crosswalk exists the
// two views can merge; until then they sit side by side and neither lies.
//
// Pure: types, row mapping and query builders. The calls live in
// ./curatedAssetsClient.

import { asBoolean, asNumber, asText, columnIndex, type Table } from "../../../shared/lib/csv";

/** [SIM] L7 bgws_platform_profile — one row per variant, land only. */
export const L7_PLATFORM_RID = "ri.foundry.main.dataset.1f5ded0d-af96-4f0e-8163-892596f5a6c7";

/** [SIM] L7 bgws_capability_profile — one row per variant per capability. */
export const L7_CAPABILITY_RID = "ri.foundry.main.dataset.2a662c29-2f41-4cd4-a887-0da0f032f895";

/** [SIM] L7 bgws_munition_profile — 150 rounds with penetration curves. */
export const L7_MUNITION_RID = "ri.foundry.main.dataset.215c86d1-4733-44a6-a086-332d8c96761b";

/** [SIM] L7 bgws_section_profile — 20 dismounted sections. */
export const L7_SECTION_RID = "ri.foundry.main.dataset.b1f7e34a-9071-4c43-92dd-5cf3abc7e12d";
/** Aircraft and warships as called-for support: 350 missions over 311 platforms. */
export const L7_SUPPORT_RID = "ri.foundry.main.dataset.f16e70e4-3158-4f2f-9904-099c2f25e725";

// ─── Types ─────────────────────────────────────────────────────────────────

/**
 * Armour at one aspect, in RHA-equivalent millimetres.
 *
 * Kinetic and chemical are separate because they diverge by a factor of three:
 * a Challenger 2's side stops 140 mm of sabot and 400 mm of shaped charge.
 * Showing one number would misinform whichever reader cared about the other.
 */
export interface Facing {
  aspect: string;
  keMm: number | null;
  ceMm: number | null;
}

export interface CuratedPlatform {
  assetId: string;
  displayName: string;
  /** land | air | sea. Aircraft and warships are browsable but not playable. */
  domain: string | null;
  /**
   * Whether BGWS can field it: land, with a move type the game resolves.
   *
   * Real and playable are different questions, and the profile answers both
   * rather than dropping what it cannot field.
   */
  playable: boolean;
  subclass: string | null;
  /**
   * The class in words, from the pipeline's own vocabulary.
   *
   * ⚠ THE LABEL BELONGS WITH THE DATA, NOT IN THE UI. The profile emits
   * `subclass_label` ("Attack Helicopter") beside the id (`attack_helo`)
   * precisely so every consumer says the same thing. A second copy of the
   * mapping here would drift the first time a class was added, so the app
   * renders this and only prettifies the id when it meets a class the
   * pipeline has not learnt.
   */
  subclassLabel: string | null;
  nation: string | null;
  baseAssetId: string | null;
  isVariant: boolean;

  moveType: string | null;
  targetClass: string | null;
  protectionBand: string | null;
  csIndex: number | null;
  capabilities: string[];

  massT: number | null;
  speedKmh: number | null;
  crewSize: number | null;
  dismountsCarried: number | null;
  dismountTimeTurns: number | null;
  opticsClass: number | null;
  signatureClass: number | null;
  requiresPrimeMover: boolean;

  facings: Facing[];
  eraFit: string | null;
  apsFit: string | null;

  bestPenMm1000m: number | null;
  bestCalibreMm: number | null;
  weaponCount: number | null;

  /** The three inputs the Combat Strength was derived from. */
  csInputPenMm: number | null;
  csInputArmourMm: number | null;
  csInputRangeM: number | null;

  protectionConfidence: string | null;
  protectionSourceRef: string | null;
  mfId: string | null;
  /** Filename in the Mildata Images media set; the path is this plus .png. */
  imageFilename: string | null;
  /**
   * "curated" | "normalised_name" | null -- how the photograph was found.
   *
   * A guessed picture should be visibly a guess. 50 of 579 assets are matched
   * by a normalised name rather than a hand-typed filename, and a wrong one is
   * then a question about the match rather than a mystery.
   */
  imageMatchMethod: string | null;

  // ── Survivability, for the assets that have no armour ────────────────────
  //
  // ⚠ NOT A SUBSTITUTE FOR protectionBand, AND NOT MILLIMETRES. Aircraft and
  // warships have no STANAG facing, so `protectionBand` and `targetClass` are
  // null for them by design. What IS known is what can reach them, which the
  // profile publishes instead.
  /** "low" | "medium" | "high", from what can engage it. */
  survivabilityBand: string | null;
  /** The weapon class that threatens it: manpads, sam_short, coastal_missile… */
  threatVulnerability: string | null;
}

export interface CuratedCapability {
  assetId: string;
  capability: string;
  maxRangeM: number | null;
  minRangeM: number | null;
  shortRangeM: number | null;
  penMm0m: number | null;
  penMm1000m: number | null;
  penMm2000m: number | null;
  hasTopAttack: boolean;
  weapons: string[];
}

export interface CuratedMunition {
  munitionId: string;
  weaponId: string | null;
  name: string;
  kind: string | null;
  isKinetic: boolean;
  isShapedCharge: boolean;
  topAttack: boolean;
  defeatsEraKe: boolean;
  /** The curve, at the ranges the game resolves at. */
  penMm0m: number | null;
  penMm1000m: number | null;
  penMm2000m: number | null;
  penMm3000m: number | null;
  lethalRadiusM: number | null;
  heFillKg: number | null;
  sourceConfidence: string | null;
  sourceRef: string | null;
}

export interface CuratedSection {
  assetId: string;
  displayName: string;
  nation: string | null;
  strength: number | null;
  fireteams: number | null;
  defaultCarrier: string | null;
  opticsClass: number | null;
  csIndex: number | null;
  capabilities: string[];
  weapons: string[];
  apersMaxRangeM: number | null;
  atkMaxRangeM: number | null;
  atmMaxRangeM: number | null;
  aaMaxRangeM: number | null;
  idfMaxRangeM: number | null;
  atkPenMm1000m: number | null;
  atmPenMm1000m: number | null;
  sourceConfidence: string | null;
  sourceRef: string | null;
}

// ─── Row mapping ───────────────────────────────────────────────────────────

/**
 * A Spark array column, as it arrives over the SQL API.
 *
 * ⚠ ALWAYS TEXT, NEVER AN ARRAY. Everything comes back through the CSV parser,
 * whose result set is `string[][]`, so an `array<string>` column is a STRING
 * containing something array-shaped. The first version of this took `unknown`
 * and branched on `Array.isArray` for a case that cannot occur; the compiler
 * rejected it for a different reason and was right twice over.
 *
 * Two shapes do occur, depending on how the column was serialised: JSON
 * (`["apers","atk"]`) and a bare comma-separated list (`apers,atk`).
 */
function asList(value: string | undefined): string[] {
  const text = asText(value);
  if (!text) return [];
  try {
    const parsed: unknown = JSON.parse(text);
    if (Array.isArray(parsed)) return parsed.map((entry) => String(entry));
  } catch {
    // Not JSON. Fall through to the comma-separated reading.
  }
  return text
    .replace(/^[[]|[\]]$/g, "")
    .split(",")
    .map((entry) => entry.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
}

const FACING_ASPECTS = [
  ["hull_front", "Hull front"],
  ["turret_front", "Turret front"],
  ["side", "Side"],
  ["rear", "Rear"],
  ["roof", "Roof"],
] as const;

export function parseCuratedPlatforms(table: Table): CuratedPlatform[] {
  const at = columnIndex(table);
  const out: CuratedPlatform[] = [];

  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const assetId = asText(get("asset_id"));
    if (!assetId) continue;

    out.push({
      assetId,
      displayName: asText(get("display_name")) ?? assetId,
      domain: asText(get("domain")),
      // The profile computes this. The fallback mirrors its rule ONLY so the
      // app tolerates a profile built before the column existed; it is not a
      // second opinion, and it can go once every branch has the column.
      playable:
        asBoolean(get("bgws_playable")) ??
        (asText(get("domain")) === "land" && asText(get("bgws_move_type")) != null),
      subclass: asText(get("subclass")),
      subclassLabel: asText(get("subclass_label")),
      nation: asText(get("nation_primary")),
      baseAssetId: asText(get("base_asset_id")),
      isVariant: asBoolean(get("is_variant")) ?? false,
      moveType: asText(get("bgws_move_type")),
      targetClass: asText(get("bgws_target_class")),
      protectionBand: asText(get("protection_band")),
      csIndex: asNumber(get("bgws_cs_index")),
      capabilities: asList(get("bgws_capabilities")),
      massT: asNumber(get("mass_combat_t")),
      speedKmh: asNumber(get("statcard_speed_kmh")),
      crewSize: asNumber(get("crew_size")),
      dismountsCarried: asNumber(get("dismounts_carried")),
      dismountTimeTurns: asNumber(get("dismount_time_turns")),
      opticsClass: asNumber(get("optics_class")),
      signatureClass: asNumber(get("signature_class")),
      requiresPrimeMover: asBoolean(get("requires_prime_mover")) ?? false,
      facings: FACING_ASPECTS.map(([column, label]) => ({
        aspect: label,
        keMm: asNumber(get(`armour_${column}_ke_mm`)),
        ceMm: asNumber(get(`armour_${column}_ce_mm`)),
      })),
      eraFit: asText(get("era_fit")),
      apsFit: asText(get("aps_fit")),
      bestPenMm1000m: asNumber(get("best_pen_mm_1000m")),
      bestCalibreMm: asNumber(get("best_calibre_mm")),
      weaponCount: asNumber(get("weapon_count")),
      csInputPenMm: asNumber(get("cs_input_pen_mm")),
      csInputArmourMm: asNumber(get("cs_input_armour_mm")),
      csInputRangeM: asNumber(get("cs_input_range_m")),
      protectionConfidence: asText(get("protection_confidence")),
      protectionSourceRef: asText(get("protection_source_ref")),
      mfId: asText(get("mf_id")),
      imageFilename: asText(get("image_filename")),
      imageMatchMethod: asText(get("image_match_method")),
      survivabilityBand: asText(get("survivability_band")),
      threatVulnerability: asText(get("bgws_threat_vulnerability")),
    });
  }
  return out;
}

// ─── Support assets ────────────────────────────────────────────────────────

/**
 * An aircraft or warship as something a commander CALLS FOR.
 *
 * ⚠ THE OTHER HALF OF THE CATALOGUE. The platform profile publishes 405 air
 * and sea rows with no move type, no armour and `playable` false, because a
 * frigate is not a counter on a 10 km land board. That is not the whole story
 * about them: they participate through response and loiter, which is what this
 * table carries and what the platform profile has no column for.
 *
 * One row per MISSION, not per platform -- an A-10 flying close air support
 * and the same A-10 flying interdiction are two rows, because they arrive at
 * different times and stay for different lengths.
 */
export interface CuratedSupport {
  assetId: string;
  platformId: string | null;
  displayName: string;
  domain: string | null;
  nation: string | null;
  /** The source's own word: ngs, cas, amphibious_lift, isr_uav… */
  missionType: string | null;
  /** What BGWS could do with it: fires | lift | recce | excluded. */
  supportRole: string | null;
  /** Turns between the call and its arrival. Always at least 1. */
  responseTurns: number | null;
  /** Turns it remains available once on station. */
  loiterTurns: number | null;
  sortiesPerDay: number | null;
  liftCapacity: number | null;
  sensorClass: string | null;
  weatherMin: string | null;
  /** What can kill it -- the counterpart to a platform's armour. */
  threatVulnerability: string | null;
  speedKph: number | null;
  rangeKm: number | null;
  confidence: string | null;
  sourceRef: string | null;
  imageFilename: string | null;
}

export function parseCuratedSupport(table: Table): CuratedSupport[] {
  const at = columnIndex(table);
  const out: CuratedSupport[] = [];

  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const assetId = asText(get("asset_id"));
    if (!assetId) continue;

    out.push({
      assetId,
      platformId: asText(get("platform_id")),
      displayName: asText(get("display_name")) ?? assetId,
      domain: asText(get("domain")),
      nation: asText(get("nation_primary")),
      missionType: asText(get("mission_type")),
      supportRole: asText(get("bgws_support_role")),
      responseTurns: asNumber(get("response_turns")),
      loiterTurns: asNumber(get("loiter_turns")),
      sortiesPerDay: asNumber(get("sorties_per_day")),
      liftCapacity: asNumber(get("lift_capacity")),
      sensorClass: asText(get("sensor_class")),
      weatherMin: asText(get("weather_min")),
      threatVulnerability: asText(get("threat_vulnerability")),
      speedKph: asNumber(get("speed_kph")),
      rangeKm: asNumber(get("range_km")),
      confidence: asText(get("confidence")),
      sourceRef: asText(get("source_ref")),
      imageFilename: asText(get("image_filename")),
    });
  }
  return out;
}

export function parseCuratedCapabilities(table: Table): CuratedCapability[] {
  const at = columnIndex(table);
  const out: CuratedCapability[] = [];
  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const assetId = asText(get("asset_id"));
    const capability = asText(get("bgws_capability"));
    if (!assetId || !capability) continue;
    out.push({
      assetId,
      capability,
      maxRangeM: asNumber(get("bgws_max_range_m")),
      minRangeM: asNumber(get("min_range_m")),
      shortRangeM: asNumber(get("short_range_m")),
      penMm0m: asNumber(get("best_pen_mm_0m")),
      penMm1000m: asNumber(get("best_pen_mm_1000m")),
      penMm2000m: asNumber(get("best_pen_mm_2000m")),
      hasTopAttack: asBoolean(get("has_top_attack")) ?? false,
      weapons: asList(get("weapons")),
    });
  }
  return out;
}

export function parseCuratedMunitions(table: Table): CuratedMunition[] {
  const at = columnIndex(table);
  const out: CuratedMunition[] = [];
  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const munitionId = asText(get("munition_id"));
    if (!munitionId) continue;
    out.push({
      munitionId,
      weaponId: asText(get("weapon_id")),
      name: asText(get("name")) ?? munitionId,
      kind: asText(get("kind")),
      isKinetic: asBoolean(get("is_kinetic")) ?? false,
      isShapedCharge: asBoolean(get("is_shaped_charge")) ?? false,
      topAttack: asBoolean(get("top_attack")) ?? false,
      defeatsEraKe: asBoolean(get("defeats_era_ke")) ?? false,
      penMm0m: asNumber(get("pen_mm_0m")),
      penMm1000m: asNumber(get("pen_mm_1000m")),
      penMm2000m: asNumber(get("pen_mm_2000m")),
      penMm3000m: asNumber(get("pen_mm_3000m")),
      lethalRadiusM: asNumber(get("lethal_radius_m")),
      heFillKg: asNumber(get("he_fill_kg")),
      sourceConfidence: asText(get("source_confidence")),
      sourceRef: asText(get("source_ref")),
    });
  }
  return out;
}

export function parseCuratedSections(table: Table): CuratedSection[] {
  const at = columnIndex(table);
  const out: CuratedSection[] = [];
  for (const row of table.rows) {
    const get = (name: string) => row[at[name]];
    const assetId = asText(get("asset_id"));
    if (!assetId) continue;
    out.push({
      assetId,
      displayName: asText(get("display_name")) ?? assetId,
      nation: asText(get("nation_primary")),
      strength: asNumber(get("strength")),
      fireteams: asNumber(get("fireteams")),
      defaultCarrier: asText(get("default_carrier")),
      opticsClass: asNumber(get("optics_class")),
      csIndex: asNumber(get("bgws_cs_index")),
      capabilities: asList(get("bgws_capabilities")),
      weapons: asList(get("weapons")),
      apersMaxRangeM: asNumber(get("apers_max_range_m")),
      atkMaxRangeM: asNumber(get("atk_max_range_m")),
      atmMaxRangeM: asNumber(get("atm_max_range_m")),
      aaMaxRangeM: asNumber(get("aa_max_range_m")),
      idfMaxRangeM: asNumber(get("idf_max_range_m")),
      atkPenMm1000m: asNumber(get("atk_pen_mm_1000m")),
      atmPenMm1000m: asNumber(get("atm_pen_mm_1000m")),
      sourceConfidence: asText(get("source_confidence")),
      sourceRef: asText(get("source_ref")),
    });
  }
  return out;
}

// ─── The penetration curve ─────────────────────────────────────────────────

export interface CurvePoint {
  rangeM: number;
  penMm: number;
}

/**
 * A munition's penetration curve, as points that can be plotted.
 *
 * ⚠ A SHAPED CHARGE PLOTS FLAT, AND THAT IS THE MOST INFORMATIVE THING THIS
 * CHART SHOWS. The jet is formed on impact, so a Kornet defeats the same
 * armour at three kilometres as at zero, while a sabot round sheds roughly a
 * twentieth of its penetration per kilometre. Put the two on the same axes
 * and the reason armies carry both becomes obvious in a way no table does.
 *
 * Nulls are dropped rather than plotted as zero: a high-explosive round has no
 * anti-armour penetration to draw, which is different from having none left.
 */
export function penetrationCurve(munition: CuratedMunition): CurvePoint[] {
  const candidates: [number, number | null][] = [
    [0, munition.penMm0m],
    [1000, munition.penMm1000m],
    [2000, munition.penMm2000m],
    [3000, munition.penMm3000m],
  ];
  return candidates
    .filter((entry): entry is [number, number] => entry[1] != null)
    .map(([rangeM, penMm]) => ({ rangeM, penMm }));
}

/**
 * Whether a curve is worth drawing.
 *
 * One point is a number, not a curve, and drawing a single dot invites the
 * reader to infer a trend from it.
 */
export function hasCurve(munition: CuratedMunition): boolean {
  return penetrationCurve(munition).length >= 2;
}

/** How much penetration is lost between the ends of the curve, as a fraction. */
export function curveFalloff(munition: CuratedMunition): number | null {
  const points = penetrationCurve(munition);
  if (points.length < 2) return null;
  const first = points[0].penMm;
  const last = points[points.length - 1].penMm;
  if (first <= 0) return null;
  return (first - last) / first;
}

// ─── Queries ───────────────────────────────────────────────────────────────

/**
 * ⚠ SELECT *, ON PURPOSE, AND THIS IS AN ARCHITECTURAL CHOICE RATHER THAN
 * LAZINESS.
 *
 * These queries used to name every column. That coupled the app's deploy to
 * the pipeline's in the most brittle possible way: adding a column to the
 * profile is a backwards-compatible change to the DATA and was a breaking
 * change to the APP, because a name in this list that the dataset does not
 * have yet fails the whole query —
 *
 *   [UNRESOLVED_COLUMN.WITH_SUGGESTION] A column ... with name
 *   `image_filename` cannot be resolved.
 *
 * — and takes the page down with it. That happened twice in one afternoon,
 * once for `image_filename` and once for `bgws_playable`, both times because
 * the app shipped ahead of the column reaching master. A process rule
 * ("merge the pipeline first") was written down after the first one and did
 * not survive the second.
 *
 * `SELECT *` removes the failure mode instead of documenting it. The row
 * mapping below is already tolerant: `columnIndex` has no entry for a column
 * that is absent, `row[undefined]` is `undefined`, and `asText`/`asNumber`
 * turn that into null. So a profile that predates a field reads as "that
 * field is unknown", which is exactly what it is, and a profile that gains a
 * field needs no app change at all.
 *
 * The cost is bandwidth for columns nothing reads. This catalogue is 223 rows;
 * it is not a consideration. If it ever becomes one, the answer is a view or
 * a narrower dataset in the pipeline, not a fragile name list here.
 */
export function buildCuratedPlatformQuery(limit = 400): string {
  return `SELECT * FROM \`${L7_PLATFORM_RID}\` ORDER BY bgws_cs_index DESC, display_name LIMIT ${limit}`;
}

export function buildCuratedCapabilityQuery(assetId: string): string {
  return (
    `SELECT * FROM \`${L7_CAPABILITY_RID}\` WHERE asset_id = ${sqlText(assetId)} ` +
    "ORDER BY bgws_capability"
  );
}


export function buildCuratedMunitionQuery(limit = 400): string {
  return `SELECT * FROM \`${L7_MUNITION_RID}\` ORDER BY pen_mm_1000m DESC NULLS LAST, name LIMIT ${limit}`;
}

export function buildCuratedSupportQuery(limit = 500): string {
  // 350 rows today. Ordered so the missions the game could act on come first
  // and `excluded` sinks to the bottom -- a reader scanning the list should
  // meet naval gunfire before they meet a strategic bomber.
  return (
    `SELECT * FROM \`${L7_SUPPORT_RID}\` ` +
    "ORDER BY bgws_support_role, response_turns, display_name " +
    `LIMIT ${limit}`
  );
}

export function buildCuratedSectionQuery(limit = 400): string {
  return `SELECT * FROM \`${L7_SECTION_RID}\` ORDER BY nation_primary, display_name LIMIT ${limit}`;
}

/** Single-quoted, with quotes doubled. Asset ids are ours, but not sanitised. */
function sqlText(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}
