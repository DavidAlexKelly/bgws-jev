// ── bgws/data/curatedAssetsClient.ts ───────────────────────────────────────
// The calls that reach Foundry for the curated L7 profiles. Everything worth
// testing is in ./curatedAssets.
//
// ⚠ These four datasets each need adding as a Resource on this app in
// Developer Console. The SQL scopes alone return 403 — see the note in
// shared/lib/sqlClient. Until they are added the curated modes render empty
// and say why, which is the same failure mode the L5 explorer already has.

import { runSql } from "../../../shared/lib/sqlClient";
import {
  buildCuratedCapabilityQuery,
  buildCuratedMunitionQuery,
  buildCuratedPlatformQuery,
  buildCuratedSectionQuery,
  buildCuratedSupportQuery,
  parseCuratedCapabilities,
  parseCuratedMunitions,
  parseCuratedPlatforms,
  parseCuratedSections,
  parseCuratedSupport,
  type CuratedCapability,
  type CuratedMunition,
  type CuratedPlatform,
  type CuratedSection,
  type CuratedSupport,
} from "./curatedAssets";

export async function loadCuratedPlatforms(
  signal?: AbortSignal,
): Promise<CuratedPlatform[]> {
  return parseCuratedPlatforms(await runSql(buildCuratedPlatformQuery(), signal));
}

export async function loadCuratedCapabilities(
  assetId: string,
  signal?: AbortSignal,
): Promise<CuratedCapability[]> {
  return parseCuratedCapabilities(
    await runSql(buildCuratedCapabilityQuery(assetId), signal),
  );
}

export async function loadCuratedMunitions(
  signal?: AbortSignal,
): Promise<CuratedMunition[]> {
  return parseCuratedMunitions(await runSql(buildCuratedMunitionQuery(), signal));
}

export async function loadCuratedSections(
  signal?: AbortSignal,
): Promise<CuratedSection[]> {
  return parseCuratedSections(await runSql(buildCuratedSectionQuery(), signal));
}

export async function loadCuratedSupport(
  signal?: AbortSignal,
): Promise<CuratedSupport[]> {
  return parseCuratedSupport(await runSql(buildCuratedSupportQuery(), signal));
}
