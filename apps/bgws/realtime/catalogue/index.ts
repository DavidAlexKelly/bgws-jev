// ── realtime/catalogue/index.ts ──────────────────────────────────────────
// The catalogue as the engine reads it: every record by id, and the checks
// that keep it complete (W14 "consistency checks", run in catalogue.test.ts).

import { GENERIC_RECORDS } from "./generic";
import type { CatalogueRecord, PlatformDomain, WeaponSpec } from "./schema";

export type { CatalogueRecord } from "./schema";

const BY_ID: ReadonlyMap<string, CatalogueRecord> = new Map(GENERIC_RECORDS.map((r) => [r.id, r]));

export const CATALOGUE: readonly CatalogueRecord[] = GENERIC_RECORDS;

/** The record with this id. Throws for an unknown id: a scenario naming a platform that does not exist is a bug, not a default. */
export function recordFor(id: string): CatalogueRecord {
  const record = BY_ID.get(id);
  if (!record) throw new Error(`Unknown catalogue record "${id}"`);
  return record;
}

export function hasRecord(id: string): boolean {
  return BY_ID.has(id);
}

export function recordsIn(domain: PlatformDomain): CatalogueRecord[] {
  return GENERIC_RECORDS.filter((r) => r.domain === domain);
}

/** A record's weapon by id. */
export function mountOf(record: CatalogueRecord, weaponId: string): WeaponSpec | undefined {
  return record.weapons.find((w) => w.id === weaponId);
}

/** Sane ranges a record's values must sit in, per domain. Every problem, worded. */
export function catalogueProblems(records: readonly CatalogueRecord[] = GENERIC_RECORDS): string[] {
  const problems: string[] = [];
  const ids = new Set<string>();
  for (const r of records) {
    const at = (what: string) => `${r.id}: ${what}`;
    if (ids.has(r.id)) problems.push(at("duplicate id"));
    ids.add(r.id);
    if (!r.rationale) problems.push(at("no rationale"));
    if (!r.template.startsWith("core/")) problems.push(at(`template "${r.template}" is not a core template`));
    if (r.body.count < 1) problems.push(at("body.count must be at least 1"));
    if (r.costUsd <= 0) problems.push(at("costUsd must be positive"));
    const m = r.mover;
    if (m.kind !== "static" && m.speedKmh <= 0) problems.push(at("a mover that moves needs a speed"));
    if (m.cruiseKmh > m.speedKmh) problems.push(at("cruise speed above best speed"));
    if (r.domain === "air" && !(m.ceilingM && m.ceilingM > 0)) problems.push(at("an aircraft needs a ceiling"));
    if (r.domain === "space" && !(m.orbitAltKm && m.orbitAltKm > 100)) problems.push(at("a satellite needs an orbit above 100 km"));
    if (r.domain === "land" && m.kind !== "foot" && m.kind !== "static" && r.stores.fuelL <= 0 && r.stores.batteryWh <= 0) {
      problems.push(at("a land vehicle needs fuel or a battery"));
    }
    if (r.stores.fuelL > 0 && r.stores.burnLph.moving <= 0 && m.kind !== "static" && m.kind !== "orbit") {
      problems.push(at("carries fuel but burns none moving"));
    }
    for (const w of r.weapons) {
      if (w.maxRangeM <= w.minRangeM) problems.push(at(`weapon ${w.id}: max range not above min range`));
      if (w.pk < 0 || w.pk > 1) problems.push(at(`weapon ${w.id}: pk outside 0–1`));
      if (w.flight !== "none" && w.speedMs <= 0) problems.push(at(`weapon ${w.id}: flies but has no speed`));
      if (w.costUsd <= 0) problems.push(at(`weapon ${w.id}: no cost`));
    }
    for (const s of r.sensors) {
      if (s.rangeM <= 0) problems.push(at(`sensor ${s.kind}: no range`));
      if (s.arcDeg <= 0 || s.arcDeg > 360) problems.push(at(`sensor ${s.kind}: arc outside 0–360`));
    }
    for (const sub of [r.carried.dismounts?.archetype, r.carried.drones?.archetype, r.carried.aircraft?.archetype]) {
      if (sub && !records.some((o) => o.id === sub)) problems.push(at(`carries unknown "${sub}"`));
    }
  }
  return problems;
}
