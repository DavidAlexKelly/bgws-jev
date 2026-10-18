// ── bgws/data/profilesClient.ts ────────────────────────────────────────────
// The two calls that actually reach Foundry. Everything they do that is worth
// testing — the queries, the row mapping — lives in ./profiles, which imports
// nothing that touches src/client.ts.

import { runSql } from "../../../shared/lib/sqlClient";
import {
  buildCapabilityQuery,
  buildPlatformQuery,
  parseCapabilityRows,
  parsePlatformRows,
  type CapabilityProfile,
  type PlatformProfile,
  type PlatformSearchOptions,
} from "./profiles";

/** Platforms a scenario author can choose from. */
export async function loadPlatformProfiles(
  options: PlatformSearchOptions = {},
  signal?: AbortSignal
): Promise<PlatformProfile[]> {
  return parsePlatformRows(await runSql(buildPlatformQuery(options), signal));
}

/**
 * The derived profile for one asset, or null when it has none.
 *
 * Null is a real answer: the L6 layer is land-only, so a ship or an aircraft
 * has no derived mobility and the explorer should say so rather than leaving
 * the source's own numbers looking authoritative.
 */
export async function loadPlatformProfile(
  assetId: string,
  signal?: AbortSignal,
): Promise<PlatformProfile | null> {
  const rows = await loadPlatformProfiles({ assetIds: [assetId], limit: 1 }, signal);
  return rows[0] ?? null;
}

/** Capabilities for a set of platforms, keyed by asset id. */
export async function loadCapabilityProfiles(
  assetIds: string[],
  signal?: AbortSignal
): Promise<Map<string, CapabilityProfile[]>> {
  const byAsset = new Map<string, CapabilityProfile[]>();
  if (assetIds.length === 0) return byAsset;

  for (const row of parseCapabilityRows(await runSql(buildCapabilityQuery(assetIds), signal))) {
    const existing = byAsset.get(row.assetId);
    if (existing) existing.push(row);
    else byAsset.set(row.assetId, [row]);
  }
  return byAsset;
}
