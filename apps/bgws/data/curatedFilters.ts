// ── bgws/data/curatedFilters.ts ────────────────────────────────────────────
// Faceted filtering for the equipment catalogue.
//
// PURE, AND SEPARATE FROM THE RENDERING, FOR THE REASON THE L5 EXPLORER
// ALREADY GIVES: "a wrong predicate does not throw, it quietly returns the
// wrong assets, and a list of tanks looks equally plausible either way."
//
// FILTERED IN THE BROWSER, NOT IN SQL, WHICH IS THE OPPOSITE OF THE L5
// EXPLORER — and deliberately. That one queries 2,258 rows across 30 facets,
// so it pushes predicates into SQL and pays a round trip per keystroke. This
// catalogue is 223 rows: 203 platforms and 20 sections. The whole thing fits
// in one query, so every facet click is instant and the counts can be exact
// rather than a second COUNT(*) that might disagree with the list.

import type { CuratedPlatform, CuratedSection } from "./curatedAssets";

/**
 * What kind of thing this is, in the terms a reader thinks in.
 *
 * The source says `domain` = land/air/sea and has a separate table for
 * sections. "Infantry" is not a domain there, but it is the first thing
 * somebody looking for a rifle section would click.
 */
export type EquipmentType = "vehicle" | "aircraft" | "naval" | "infantry";

export const TYPE_LABEL: Record<EquipmentType, string> = {
  vehicle: "vehicle",
  aircraft: "aircraft",
  naval: "naval",
  infantry: "infantry",
};

/** One row in the unified list, whatever table it came from. */
export interface EquipmentRow {
  id: string;
  name: string;
  type: EquipmentType;
  subclass: string | null;
  /**
   * The class in words, from the pipeline's `subclass_label`.
   *
   * The facet filters on `subclass` (the stable id) and DISPLAYS this, so a
   * reader sees "Attack Helicopter" while the selection is still keyed on
   * `attack_helo`. Null where the pipeline has not labelled a class, and the
   * facet then falls back to the id rather than showing nothing.
   */
  subclassLabel: string | null;
  nation: string | null;
  capabilities: string[];
  csIndex: number | null;
  /** Null for sections and for anything with no armour model. */
  frontArmourMm: number | null;
  playable: boolean;
  /** The underlying record, for the detail pane. */
  platform?: CuratedPlatform;
  section?: CuratedSection;
}

function typeForDomain(domain: string | null): EquipmentType {
  if (domain === "air") return "aircraft";
  if (domain === "sea") return "naval";
  return "vehicle";
}

export function rowsFromPlatforms(platforms: CuratedPlatform[]): EquipmentRow[] {
  return platforms.map((platform) => ({
    id: platform.assetId,
    name: platform.displayName,
    type: typeForDomain(platform.domain),
    subclass: platform.subclass,
    subclassLabel: platform.subclassLabel,
    nation: platform.nation,
    capabilities: platform.capabilities,
    csIndex: platform.csIndex,
    frontArmourMm: platform.facings.find((f) => f.aspect === "Hull front")?.keMm ?? null,
    playable: platform.playable,
    platform,
  }));
}

export function rowsFromSections(sections: CuratedSection[]): EquipmentRow[] {
  return sections.map((section) => ({
    id: section.assetId,
    name: section.displayName,
    type: "infantry" as EquipmentType,
    // Sections have no class in the source. "section" is ours, and it keeps
    // the subclass facet from showing a blank entry for twenty rows.
    subclass: "section",
    subclassLabel: "Infantry Section",
    nation: section.nation,
    capabilities: section.capabilities,
    csIndex: section.csIndex,
    frontArmourMm: null,
    playable: true,
    section,
  }));
}

// ─── The filter model ──────────────────────────────────────────────────────

export interface CuratedFilters {
  search?: string;
  types?: EquipmentType[];
  subclasses?: string[];
  nations?: string[];
  capabilities?: string[];
  /** Only what the game can field. Off by default: an explorer shows all. */
  playableOnly?: boolean;
  /** Only rows with an armour model, which excludes aircraft and ships. */
  armouredOnly?: boolean;
}

/**
 * Whether a row survives the filters.
 *
 * ⚠ FACETS ARE OR WITHIN A GROUP AND AND BETWEEN GROUPS, which is the
 * convention the L5 explorer uses and the one a reader expects: picking `mbt`
 * and `ifv` widens the list, picking `mbt` and nation `Russia` narrows it. The
 * opposite reading — AND within a group — makes every second click return
 * nothing, which reads as a broken filter rather than an empty set.
 *
 * An empty or absent group means "no opinion", never "match nothing".
 */
export function matchesCurated(row: EquipmentRow, filters: CuratedFilters): boolean {
  const needle = filters.search?.trim().toLowerCase();
  if (needle) {
    // The label is searched as well as the id, so "attack helicopter" finds
    // what the reader can see on screen and "attack_helo" still works.
    const haystack =
      `${row.name} ${row.subclass ?? ""} ${row.subclassLabel ?? ""} ${row.nation ?? ""}`.toLowerCase();
    if (!haystack.includes(needle)) return false;
  }

  if (filters.types?.length && !filters.types.includes(row.type)) return false;
  if (filters.subclasses?.length && !filters.subclasses.includes(row.subclass ?? "")) {
    return false;
  }
  if (filters.nations?.length && !filters.nations.includes(row.nation ?? "")) return false;

  if (filters.capabilities?.length) {
    // A row matches if it has ANY of the selected capabilities. "Show me
    // everything that can engage armour" is a union, not an intersection.
    const has = filters.capabilities.some((capability) =>
      row.capabilities.includes(capability),
    );
    if (!has) return false;
  }

  if (filters.playableOnly && !row.playable) return false;
  if (filters.armouredOnly && row.frontArmourMm == null) return false;

  return true;
}

export function applyCurated(
  rows: EquipmentRow[],
  filters: CuratedFilters,
): EquipmentRow[] {
  return rows.filter((row) => matchesCurated(row, filters));
}

// ─── Facet counts ──────────────────────────────────────────────────────────

export interface FacetEntry {
  value: string;
  label: string;
  count: number;
  selected: boolean;
}

/**
 * Counts for one facet, computed against the list filtered by EVERY OTHER
 * facet.
 *
 * ⚠ NOT AGAINST THE FULLY FILTERED LIST, AND THE DIFFERENCE IS THE ONE THAT
 * MAKES FACETS USABLE. If a facet's own selection were applied before
 * counting, every unselected value in that group would read zero and the
 * reader could never see what else was available to widen to. Counting
 * against the others is what lets "mbt 16" and "ifv 9" both show while `mbt`
 * is selected.
 */
export function facetCounts(
  rows: EquipmentRow[],
  filters: CuratedFilters,
  facet: "types" | "subclasses" | "nations" | "capabilities",
): FacetEntry[] {
  const others: CuratedFilters = { ...filters, [facet]: undefined };
  const pool = applyCurated(rows, others);
  const selected = new Set(filters[facet] ?? []);
  const counts = new Map<string, number>();
  // value -> the words to show for it. Filtering stays keyed on the stable id;
  // only the rendering changes, so a selection survives a label being edited.
  const labels = new Map<string, string>();

  for (const row of pool) {
    const values =
      facet === "types"
        ? [row.type as string]
        : facet === "subclasses"
          ? [row.subclass ?? ""]
          : facet === "nations"
            ? [row.nation ?? ""]
            : row.capabilities;
    for (const value of values) {
      if (!value) continue;
      counts.set(value, (counts.get(value) ?? 0) + 1);
      if (facet === "subclasses" && row.subclassLabel) {
        labels.set(value, row.subclassLabel);
      }
    }
  }

  // Selected values are kept even at zero, so a selection never silently
  // disappears from the panel it was made in.
  for (const value of selected) {
    if (!counts.has(value)) counts.set(value, 0);
  }

  return [...counts.entries()]
    .map(([value, count]) => ({
      value,
      // The pipeline's words where it has them, the raw id where it does not.
      // A class the vocabulary has not learnt still shows SOMETHING.
      label: labels.get(value) ?? value,
      count,
      selected: selected.has(value),
    }))
    .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
}

/** Toggle one value in one facet group, returning a new filter object. */
export function toggleCurated<K extends "types" | "subclasses" | "nations" | "capabilities">(
  filters: CuratedFilters,
  facet: K,
  value: string,
): CuratedFilters {
  const current = (filters[facet] ?? []) as string[];
  const next = current.includes(value)
    ? current.filter((entry) => entry !== value)
    : [...current, value];
  return { ...filters, [facet]: next.length ? next : undefined };
}

/** How many facet choices are active, for the "clear" affordance. */
export function activeCuratedCount(filters: CuratedFilters): number {
  return (
    (filters.types?.length ?? 0) +
    (filters.subclasses?.length ?? 0) +
    (filters.nations?.length ?? 0) +
    (filters.capabilities?.length ?? 0) +
    (filters.playableOnly ? 1 : 0) +
    (filters.armouredOnly ? 1 : 0) +
    (filters.search?.trim() ? 1 : 0)
  );
}
