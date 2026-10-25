// ── realtime/catalogue/snapshots.ts ──────────────────────────────────────
// The catalogue's land records as platforms a force list can name.
//
// A land unit on the board is a ForceElement, built from a PlatformSnapshot,
// so the land engine fights it exactly as it fights a sourced tank. The
// generic records (generic.ts) become snapshots here: their direct-fire
// weapons become the capabilities the shared fire table reads, and the rest
// of the record — stores, indirect fire, air defence, drones, emitters — is
// read by the world layer (realtime/world) from the record itself.
//
// Every figure is declared: `valuesDeclared` says so, as the force-list
// convention requires for a platform the source does not model.

import type { Capability } from "../../lib/state";
import type { MoveType, TargetClass } from "../../data/profiles";
import type { PlatformSnapshot } from "../../rules/forceList";
import { CATALOGUE } from "./index";
import type { CatalogueRecord } from "./schema";

/** The direct-fire cap the land engine plays at. */
const DIRECT_CAP_M = 3000;

function moveTypeOf(record: CatalogueRecord): MoveType {
  return record.mover.kind === "foot" ? "F" : record.mover.kind === "wheeled" ? "W" : "T";
}

function targetClassOf(record: CatalogueRecord): TargetClass {
  if (record.body.vulnerability === "personnel") return "foot";
  return record.body.vulnerability === "armoured" || record.body.armourMm >= 100 ? "armoured_vehicle" : "soft_skin";
}

/** What the shared fire table can fire from this record: direct-fire weapons only. */
function capabilitiesOf(record: CatalogueRecord): Capability[] {
  const caps: Capability[] = [];
  const ammo = record.stores.ammo;
  for (const w of record.weapons) {
    if (w.kind === "directFire") {
      caps.push({ kind: "atk", munition: "ke", maxRangeM: Math.min(DIRECT_CAP_M, w.maxRangeM), shortRangeM: 1500, penetrationMm: w.penetrationMm, rofSustained: 6 });
    } else if (w.kind === "atgm") {
      caps.push({ kind: "atm", munition: "ceTandem", topAttack: w.topAttack, maxRangeM: Math.min(DIRECT_CAP_M, w.maxRangeM), shortRangeM: 1500, penetrationMm: w.penetrationMm, rofSustained: 2 });
    }
  }
  if (ammo.cannon30) caps.push({ kind: "atk", munition: "ke", maxRangeM: 2000, shortRangeM: 1000, penetrationMm: 90, rofSustained: 30 });
  if (ammo.smallArms) caps.push({ kind: "apers", maxRangeM: record.mover.kind === "foot" ? 600 : 1000, shortRangeM: 300 });
  // Anything with no weapon of its own can still defend itself, badly, against people on foot.
  if (caps.length === 0) caps.push({ kind: "apers", maxRangeM: 300, shortRangeM: 150 });
  return caps;
}

/** A combat-strength index for one platform of this record, on the force lists' scale (a tank is about 8.5). */
function csIndexOf(record: CatalogueRecord): number {
  const caps = capabilitiesOf(record);
  const anti = caps.some((c) => c.kind === "atk" || c.kind === "atm");
  const protection = record.body.armourMm >= 400 ? 3 : record.body.armourMm >= 100 ? 1.5 : 0.5;
  return Math.max(1, Math.round((anti ? 4 : 1) + protection + (record.body.vulnerability === "personnel" ? 1 : 0)));
}

export function snapshotFromRecord(record: CatalogueRecord): PlatformSnapshot {
  return {
    assetId: record.id,
    displayName: record.name,
    moveType: moveTypeOf(record),
    targetClass: targetClassOf(record),
    csIndex: csIndexOf(record),
    protectionBand: targetClassOf(record),
    ...(record.body.armourMm > 0 ? { armourMm: record.body.armourMm } : {}),
    speedKmh: record.mover.speedKmh,
    capabilities: capabilitiesOf(record),
    valuesDeclared: `a generic catalogue record (realtime/catalogue): ${record.rationale}`,
  };
}

/** Every land record, as a placeable platform, keyed by record id ("generic/mbt"). */
export const GENERIC_LAND_SNAPSHOTS: Readonly<Record<string, PlatformSnapshot>> = Object.fromEntries(
  CATALOGUE.filter((r) => r.domain === "land").map((r) => [r.id, snapshotFromRecord(r)]),
);

/** A generic catalogue record's symbol for a side (2525), for a unit placed as that type. */
export function genericSidc(platform: string, side: "blue" | "red"): string | undefined {
  return CATALOGUE.find((r) => r.id === platform)?.sidc[side];
}
