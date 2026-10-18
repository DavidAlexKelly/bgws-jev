// ── bgws/data/assetCardsClient.ts ──────────────────────────────────────────
// The calls that reach Foundry. Everything worth testing is in ./assetCards,
// ./assetFilters and ./unitImageIndex.

import { runSql } from "../../../shared/lib/sqlClient";
import { parseAssetCards, type AssetCard } from "./assetCards";
import {
  buildAssetQuery,
  buildFacetCountsQuery,
  buildMatchCountQuery,
  parseFacetCounts,
  type AssetFilters,
  type FacetCount,
} from "./assetFilters";
import {
  buildImageCountQuery,
  buildImageIndexQuery,
  indexByAssetId,
  parseCount,
  parseImageIndex,
  type UnitImageRow,
} from "./unitImageIndex";

export async function loadAssetCards(
  filters: AssetFilters = {},
  signal?: AbortSignal,
): Promise<AssetCard[]> {
  return parseAssetCards(await runSql(buildAssetQuery(filters), signal));
}

/**
 * How many rows match, ignoring the limit.
 *
 * Separate from loadAssetCards so the list can render as soon as the page
 * arrives rather than waiting on a COUNT over the whole catalogue.
 */
export async function loadMatchCount(
  filters: AssetFilters = {},
  signal?: AbortSignal,
): Promise<number> {
  return parseCount(await runSql(buildMatchCountQuery(filters), signal));
}

export async function loadFacetCounts(signal?: AbortSignal): Promise<FacetCount[]> {
  return parseFacetCounts(await runSql(buildFacetCountsQuery(), signal));
}

export async function loadImageCount(signal?: AbortSignal): Promise<number> {
  return parseCount(await runSql(buildImageCountQuery(), signal));
}

/** The image rows for the assets currently on screen, keyed by asset id. */
export async function loadImageIndex(
  assetIds: string[],
  signal?: AbortSignal,
): Promise<Map<string, UnitImageRow>> {
  const query = buildImageIndexQuery(assetIds);
  if (!query) return new Map();
  return indexByAssetId(parseImageIndex(await runSql(query, signal)));
}
