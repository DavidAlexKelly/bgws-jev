// ── bgws/data/assetFilters.ts ──────────────────────────────────────────────
// The explorer's filter model: what can be filtered, and the SQL for it.
//
// Pure. No import here reaches Foundry, so all of it is testable — which
// matters, because every bug in this file is a silent one. A wrong predicate
// does not throw; it just quietly returns the wrong assets, and a list of
// tanks looks equally plausible either way.
//
// THREE THINGS THIS FILE IS DELIBERATE ABOUT:
//
// 1. NULL IS A VALUE. 387 assets have no armour class, 261 no nation, 52 no
//    subclass. `nation IN ('germany')` and `nation IS NULL` are different
//    predicates and a chip for "none" has to generate the second one. Folding
//    them together is the obvious bug and it is invisible.
//
// 2. THE SAME EXPRESSION BUILDS THE FILTER AND THE COUNT. The chips show
//    counts; clicking one filters. If those two are built by different code
//    the counts stop matching the results, usually only for the rows nobody
//    checks. `canonicalExpr` is shared by both for that reason.
//
// 3. THE NATION COLUMN IS DIRTY AND WE SAY SO. The source carries both
//    `united_kingdom` (267) and `britain` (22), and both `united_states`
//    (224) and `usa` (15). They are the same nation. NATION_ALIASES folds
//    them, in the open, where it can be read and argued with — rather than
//    leaving a reader to wonder why Britain has two entries and neither is
//    the real total.

import { asNumber, asText, columnIndex, type Table } from "../../../shared/lib/csv";
import { sqlLiteral } from "../../../shared/lib/sql";
import { ASSET_CARD_RID, ASSET_CARD_COLUMNS } from "./assetCards";
import { PLATFORM_PROFILE_RID } from "./profiles";
import { UNIT_IMAGE_INDEX_RID } from "./unitImageIndex";

/**
 * The bucket for rows where the facet column is null.
 *
 * Chosen to be something no real value could collide with. If the source ever
 * grows a nation literally called "(none)" this breaks, which is why it is a
 * constant rather than a string typed in four places.
 */
export const NULL_FACET = "(none)";

/** Facets: low-cardinality string columns worth offering as chips. */
export type FacetKey =
  | "domain"
  | "subclass"
  | "nation"
  | "armourClass"
  | "weaponClass"
  | "variantClass";

export const FACET_COLUMN: Record<FacetKey, string> = {
  domain: "domain",
  subclass: "subclass",
  nation: "nation_primary",
  armourClass: "armor_class_primary",
  weaponClass: "primary_weapon_class",
  variantClass: "variant_class",
};

export const FACET_LABEL: Record<FacetKey, string> = {
  domain: "Domain",
  subclass: "Subclass",
  nation: "Nation",
  armourClass: "Armour class",
  weaponClass: "Weapon class",
  variantClass: "Variant class",
};

/**
 * Values in the source that mean the same thing.
 *
 * Key is the value we keep; the array lists every raw value that folds into
 * it, including the key itself. Only `nation` needs this so far.
 */
export const NATION_ALIASES: Record<string, string[]> = {
  united_kingdom: ["united_kingdom", "britain"],
  united_states: ["united_states", "usa"],
};

const FACET_ALIASES: Partial<Record<FacetKey, Record<string, string[]>>> = {
  nation: NATION_ALIASES,
};

/** Booleans. Each can be required true, required false, or left alone. */
export type FlagKey =
  | "armed"
  | "mobile"
  | "variant"
  | "engagesAir"
  | "engagesGround"
  | "fullTraverse";

const FLAG_COLUMN: Record<FlagKey, string> = {
  armed: "is_armed",
  mobile: "is_mobile",
  variant: "is_variant",
  engagesAir: "engages_air",
  engagesGround: "engages_ground",
  fullTraverse: "has_full_traverse",
};

export const FLAG_LABEL: Record<FlagKey, string> = {
  armed: "Armed",
  mobile: "Mobile",
  variant: "Variant",
  engagesAir: "Engages air",
  engagesGround: "Engages ground",
  fullTraverse: "Full traverse",
};

/**
 * Flags answered by another dataset rather than a column here.
 *
 * `hasImage` asks the image index; `inWargame` asks whether the L6 land
 * profile has a row. Both are membership tests, so both are a subquery.
 */
export type ExistsKey = "hasImage" | "inWargame";

const EXISTS_SOURCE: Record<ExistsKey, string> = {
  hasImage: UNIT_IMAGE_INDEX_RID,
  inWargame: PLATFORM_PROFILE_RID,
};

export const EXISTS_LABEL: Record<ExistsKey, string> = {
  hasImage: "Has image",
  inWargame: "In wargame (L6)",
};

/** Numeric columns worth a min/max box. */
export type RangeKey =
  | "massT"
  | "armourMaxMm"
  | "calibreMm"
  | "crewSize"
  | "engagementRangeM"
  | "year";

const RANGE_COLUMN: Record<RangeKey, string> = {
  massT: "mass_t",
  armourMaxMm: "armor_max_mm",
  calibreMm: "primary_caliber_mm",
  crewSize: "crew_size",
  engagementRangeM: "max_engagement_range_m",
  year: "year_from",
};

export const RANGE_LABEL: Record<RangeKey, string> = {
  massT: "Mass (t)",
  armourMaxMm: "Armour (mm)",
  calibreMm: "Calibre (mm)",
  crewSize: "Crew",
  engagementRangeM: "Range (m)",
  year: "Year from",
};

/**
 * Warnings for ranges whose underlying column is not what a reader assumes.
 *
 * `mass_t` on the L5 card is the source's ROOT mass field, which is a
 * template placeholder for a large part of the catalogue -- every Leopard 2
 * variant reads exactly 47 t, from the A4 to the A5 PSO, and 140 unrelated
 * assets share 5.5 t. Filtering 40-70 t therefore returns a plausible list
 * for the wrong reason, and silently omits assets whose real mass is in range.
 *
 * The trustworthy figure is `mass_combat_t` on the L6 profile, derived from
 * VehiclePhys.Mass.TakeOff, and it only exists for the 1,239 land platforms.
 * Rather than quietly filter on one and display the other, the filter says so.
 */
export const RANGE_WARNING: Partial<Record<RangeKey, string>> = {
  massT:
    "This is the source's root mass field, a template placeholder for much of " +
    "the catalogue — every Leopard 2 reads 47 t. Real combat mass is on the L6 " +
    "profile in the detail pane.",
};

export interface NumericRange {
  min?: number;
  max?: number;
}

export type SortKey =
  | "name"
  | "massDesc"
  | "armourDesc"
  | "calibreDesc"
  | "completenessDesc";

const SORT_SQL: Record<SortKey, string> = {
  name: "display_name",
  // NULLS LAST throughout: an asset with no mass is not the heaviest thing in
  // the catalogue, and a descending sort would otherwise put it first.
  massDesc: "mass_t DESC NULLS LAST, display_name",
  armourDesc: "armor_max_mm DESC NULLS LAST, display_name",
  calibreDesc: "primary_caliber_mm DESC NULLS LAST, display_name",
  completenessDesc: "data_completeness_score DESC NULLS LAST, display_name",
};

export const SORT_LABEL: Record<SortKey, string> = {
  name: "Name",
  massDesc: "Heaviest",
  armourDesc: "Thickest armour",
  calibreDesc: "Biggest gun",
  completenessDesc: "Most complete",
};

export interface AssetFilters {
  /** Matches display name or asset id, case-insensitively. */
  search?: string;
  /** Facet to selected values. An absent or empty array means no constraint. */
  facets?: Partial<Record<FacetKey, string[]>>;
  flags?: Partial<Record<FlagKey, boolean>>;
  exists?: Partial<Record<ExistsKey, boolean>>;
  ranges?: Partial<Record<RangeKey, NumericRange>>;
  sort?: SortKey;
  limit?: number;
}

export const EMPTY_FILTERS: AssetFilters = {};

export const FACET_KEYS = Object.keys(FACET_COLUMN) as FacetKey[];
export const FLAG_KEYS = Object.keys(FLAG_COLUMN) as FlagKey[];
export const EXISTS_KEYS = Object.keys(EXISTS_SOURCE) as ExistsKey[];
export const RANGE_KEYS = Object.keys(RANGE_COLUMN) as RangeKey[];

/**
 * The canonical value of a facet column, as SQL.
 *
 * Folds aliases and turns null into NULL_FACET, so that grouping by this
 * expression and filtering on it agree by construction.
 */
export function canonicalExpr(facet: FacetKey): string {
  const column = FACET_COLUMN[facet];
  const aliases = FACET_ALIASES[facet];

  let value = column;
  if (aliases) {
    const whens = Object.entries(aliases)
      // A canonical value whose only alias is itself needs no CASE arm.
      .filter(([canonical, raw]) => raw.some((one) => one !== canonical))
      .map(
        ([canonical, raw]) =>
          `WHEN ${column} IN (${raw.map(sqlLiteral).join(", ")}) THEN ${sqlLiteral(canonical)}`,
      );
    if (whens.length) value = `CASE ${whens.join(" ")} ELSE ${column} END`;
  }

  return `COALESCE(${value}, ${sqlLiteral(NULL_FACET)})`;
}

/** Every predicate implied by a set of filters. */
export function filterPredicates(filters: AssetFilters): string[] {
  const where: string[] = ["asset_id IS NOT NULL"];

  if (filters.search?.trim()) {
    const term = sqlLiteral(`%${filters.search.trim().toLowerCase()}%`);
    // Asset id as well as display name: the ids are readable paths and
    // searching "tankmodels" is a reasonable thing for someone to try.
    where.push(`(LOWER(display_name) LIKE ${term} OR LOWER(asset_id) LIKE ${term})`);
  }

  for (const facet of FACET_KEYS) {
    const selected = filters.facets?.[facet];
    if (!selected?.length) continue;
    where.push(`${canonicalExpr(facet)} IN (${selected.map(sqlLiteral).join(", ")})`);
  }

  for (const flag of FLAG_KEYS) {
    const required = filters.flags?.[flag];
    if (required === undefined) continue;
    // COALESCE, because a null flag means "the source did not say" and the
    // rest of this app already reads that as false. `col = false` would drop
    // those rows from BOTH sides of the filter, which is the one answer that
    // is certainly wrong.
    where.push(`COALESCE(${FLAG_COLUMN[flag]}, false) = ${required}`);
  }

  for (const key of EXISTS_KEYS) {
    const required = filters.exists?.[key];
    if (required === undefined) continue;
    // `IS NOT NULL` inside the subquery is not decoration: a single null in
    // an NOT IN list makes the whole predicate unknown and returns nothing.
    const subquery =
      `SELECT asset_id FROM \`${EXISTS_SOURCE[key]}\` WHERE asset_id IS NOT NULL`;
    where.push(`asset_id ${required ? "IN" : "NOT IN"} (${subquery})`);
  }

  for (const key of RANGE_KEYS) {
    const range = filters.ranges?.[key];
    if (!range) continue;
    const column = RANGE_COLUMN[key];
    // A range excludes rows with no value, which is why the UI says so. Most
    // of this catalogue has no mass at all (1,132 of 2,258 do).
    if (range.min !== undefined && Number.isFinite(range.min)) {
      where.push(`${column} >= ${range.min}`);
    }
    if (range.max !== undefined && Number.isFinite(range.max)) {
      where.push(`${column} <= ${range.max}`);
    }
  }

  return where;
}

export function buildAssetQuery(filters: AssetFilters = {}): string {
  return [
    `SELECT ${ASSET_CARD_COLUMNS}`,
    `FROM \`${ASSET_CARD_RID}\``,
    `WHERE ${filterPredicates(filters).join("\n  AND ")}`,
    `ORDER BY ${SORT_SQL[filters.sort ?? "name"]}`,
    `LIMIT ${filters.limit ?? 300}`,
  ].join("\n");
}

/** How many rows match, ignoring the limit — so the UI can say "of 412". */
export function buildMatchCountQuery(filters: AssetFilters = {}): string {
  return [
    "SELECT COUNT(*) AS n",
    `FROM \`${ASSET_CARD_RID}\``,
    `WHERE ${filterPredicates(filters).join("\n  AND ")}`,
  ].join("\n");
}

export interface FacetCount {
  facet: FacetKey;
  value: string;
  count: number;
}

/**
 * Every facet's values and counts, in one query.
 *
 * Counts are over the WHOLE catalogue, not the current filter. That is a
 * choice: filtered facet counts have to exclude their own facet's predicate
 * to be useful, and a half-done version of that ("germany 367" becoming
 * "germany 367, everything else 0" the moment you click it) is worse than a
 * stable total. The header reports how many rows actually matched.
 */
export function buildFacetCountsQuery(): string {
  const parts = FACET_KEYS.map((facet) =>
    [
      `SELECT ${sqlLiteral(facet)} AS facet, ${canonicalExpr(facet)} AS value, COUNT(*) AS n`,
      `FROM \`${ASSET_CARD_RID}\``,
      `GROUP BY ${canonicalExpr(facet)}`,
    ].join("\n"),
  );
  return `${parts.join("\nUNION ALL\n")}\nORDER BY facet, n DESC`;
}

export function parseFacetCounts(table: Table): FacetCount[] {
  const at = columnIndex(table);
  const known = new Set<string>(FACET_KEYS);

  return table.rows
    .map((row) => ({
      facet: asText(row[at.facet]) as FacetKey,
      value: asText(row[at.value]) ?? NULL_FACET,
      count: asNumber(row[at.n]) ?? 0,
    }))
    // A facet name we do not recognise means the query and this parser have
    // drifted apart; dropping it beats rendering an untitled chip group.
    .filter((entry) => known.has(entry.facet) && entry.count > 0);
}

/** Group by facet, preserving the query's descending-count order. */
export function groupFacetCounts(counts: FacetCount[]): Record<FacetKey, FacetCount[]> {
  const grouped = {} as Record<FacetKey, FacetCount[]>;
  for (const facet of FACET_KEYS) grouped[facet] = [];
  for (const entry of counts) grouped[entry.facet].push(entry);
  return grouped;
}

/** For the "clear" button's badge, and to decide whether to show it at all. */
export function activeFilterCount(filters: AssetFilters): number {
  let total = 0;
  if (filters.search?.trim()) total += 1;
  for (const facet of FACET_KEYS) total += filters.facets?.[facet]?.length ? 1 : 0;
  for (const flag of FLAG_KEYS) total += filters.flags?.[flag] === undefined ? 0 : 1;
  for (const key of EXISTS_KEYS) total += filters.exists?.[key] === undefined ? 0 : 1;
  for (const key of RANGE_KEYS) {
    const range = filters.ranges?.[key];
    if (range && (range.min !== undefined || range.max !== undefined)) total += 1;
  }
  return total;
}

/** Add or remove one value from a facet, returning a new filter set. */
export function toggleFacetValue(
  filters: AssetFilters,
  facet: FacetKey,
  value: string,
): AssetFilters {
  const current = filters.facets?.[facet] ?? [];
  const next = current.includes(value)
    ? current.filter((one) => one !== value)
    : [...current, value];

  const facets = { ...filters.facets, [facet]: next };
  if (!next.length) delete facets[facet];
  return { ...filters, facets };
}

/** Cycle a flag: unset to true, true to false, false back to unset. */
export function cycleFlag(filters: AssetFilters, flag: FlagKey): AssetFilters {
  const current = filters.flags?.[flag];
  const next = current === undefined ? true : current ? false : undefined;

  const flags = { ...filters.flags, [flag]: next };
  if (next === undefined) delete flags[flag];
  return { ...filters, flags };
}

export function cycleExists(filters: AssetFilters, key: ExistsKey): AssetFilters {
  const current = filters.exists?.[key];
  const next = current === undefined ? true : current ? false : undefined;

  const exists = { ...filters.exists, [key]: next };
  if (next === undefined) delete exists[key];
  return { ...filters, exists };
}

export function setRange(
  filters: AssetFilters,
  key: RangeKey,
  range: NumericRange,
): AssetFilters {
  const ranges = { ...filters.ranges, [key]: range };
  if (range.min === undefined && range.max === undefined) delete ranges[key];
  return { ...filters, ranges };
}
