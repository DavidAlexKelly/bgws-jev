// ── realtime/model/archetypes.ts ─────────────────────────────────────────
// The archetype table (MULTI_DOMAIN_PLAN.md W2): from what a platform's data
// says it is to a generic catalogue record. The archetype picks the default
// decision graph and fills every component the data does not have.
//
// A platform placed by its catalogue id ("generic/fpvTeamRf") is that
// archetype. A sourced or pinned platform (a Challenger troop, the declared
// mortar section) is matched on its data: indirect fire, target class, move
// type, its weapons and armour, and whether it is a headquarters.

import { PLATFORM_SNAPSHOT, type PlatformSnapshot } from "../../rules/forceList";
import { hasRecord } from "../catalogue";
import { GENERIC_LAND_SNAPSHOTS } from "../catalogue/snapshots";

export type ArchetypeId = string;

/** The archetype for a platform key, with or without its snapshot. */
export function archetypeFor(platform: string, extra: { commandRating?: number } = {}): ArchetypeId {
  if ((extra.commandRating ?? 0) > 0) return "generic/cp-company";
  if (hasRecord(platform)) return platform;
  const snap: PlatformSnapshot | undefined = PLATFORM_SNAPSHOT[platform] ?? GENERIC_LAND_SNAPSHOTS[platform];
  if (!snap) return "generic/infantrySection";
  return archetypeOfSnapshot(platform, snap);
}

export function archetypeOfSnapshot(key: string, snap: Pick<PlatformSnapshot, "targetClass" | "moveType" | "capabilities" | "armourMm" | "displayName">): ArchetypeId {
  const name = `${key} ${snap.displayName}`.toLowerCase();
  const kinds = new Set(snap.capabilities.map((c) => c.kind));
  if (kinds.has("idf") || name.includes("mortar")) return "generic/mortar120";
  if (kinds.has("aa")) return "generic/shorad";
  if (snap.targetClass === "foot") {
    return /javelin|_at\b|atgm|anti-tank/.test(name) ? "generic/atgmTeam" : "generic/infantrySection";
  }
  if (snap.targetClass === "armoured_vehicle") {
    const bestPen = Math.max(0, ...snap.capabilities.map((c) => c.penetrationMm ?? 0));
    const heavy = (snap.armourMm ?? 0) >= 400 || (kinds.has("atk") && bestPen >= 450 && snap.moveType === "T");
    if (/bmp|btr|warrior|bradley|stryker|ifv|apc|cv90|marder|puma/.test(name)) return "generic/ifv";
    return heavy ? "generic/mbt" : "generic/ifv";
  }
  return "generic/recce";
}
