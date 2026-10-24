// ── realtime/presets.ts ──────────────────────────────────────────────────
// The milestone scenarios (MULTI_DOMAIN_PLAN.md §7): one playable scenario
// for each milestone, exercising everything built up to it. Each is an
// ordinary scenario file (scenario.ts), so it can be downloaded, edited and
// shared like any other, and played headless (`npm run bgws:run`).

import { offsetToLatLng, DEFAULT_ORIGIN, type PlacedElement } from "../lib/forceBuilder";
import type { LatLng } from "../lib/board";
import type { Side } from "../lib/state";
import type { TroopQualityName } from "../rules/forceList";
import { SCENARIO_FORMAT, SCENARIO_VERSION, type RealtimeScenario, type ScenarioWorld } from "./scenario";
import { DOMAINS, scopeFor, type Domain, type DomainMode, type ScenarioType, type Scope } from "./scope";
import type { EntitySpec, HqSpec, ObstacleSpec } from "./world/create";

const O = DEFAULT_ORIGIN;
export const p = (east: number, north: number, origin: LatLng = O): LatLng => offsetToLatLng(origin, east, north);

function unit(id: string, side: Side, platform: string, count: number, east: number, north: number, label?: string, quality: TroopQualityName = "regular", commandRating?: number): PlacedElement {
  return { id, label: label ?? id, side, platform, platformCount: count, troopQuality: quality, position: p(east, north), ...(commandRating ? { commandRating } : {}) };
}

function entity(id: string, owner: EntitySpec["owner"], archetype: string, east: number, north: number, extra: Partial<EntitySpec> = {}): EntitySpec {
  return { id, owner, archetype, at: p(east, north), ...extra };
}

function obstacle(owner: Side, kind: ObstacleSpec["kind"], east: number, north: number, widthM: number, depthM: number, density = 0.5): ObstacleSpec {
  return { owner, kind, at: p(east, north), widthM, depthM, bearingDeg: 0, density };
}

function scopeOf(type: ScenarioType, durationMin: number, domains: Partial<Record<Domain, DomainMode>>, over: Partial<Scope> = {}): Scope {
  const base = scopeFor(type);
  return { ...base, ...over, durationMin, domains: { ...(Object.fromEntries(DOMAINS.map((d) => [d, base.domains[d]])) as Record<Domain, DomainMode>), ...domains } };
}

function scenario(name: string, seed: string, scope: Scope, forces: PlacedElement[], world: ScenarioWorld, comms: "radio" | "perfect" = "radio"): RealtimeScenario {
  return {
    format: SCENARIO_FORMAT,
    version: SCENARIO_VERSION,
    name,
    savedAt: "2026-10-24T00:00:00.000Z",
    seed,
    ground: { choice: "generated", seed: `${seed}-ground` },
    scope,
    rules: { comms, breakpointPct: 50 },
    command: { useCommander: false, model: "claude-sonnet-5-5", useJev: false, directives: { blue: "", red: "" } },
    forces,
    behaviours: {},
    behaviourTemplates: [],
    world,
  };
}

/**
 * M2 — Combined-arms company: a mechanised company's hasty attack on a dug-in
 * platoon behind a minefield. Dismounts, mortars on call, a breach, and
 * ammunition and fuel drawn down (W2, W3).
 */
export function mechCompanyAttack(): RealtimeScenario {
  return scenario(
    "M2 — Mech company hasty attack",
    "m2-hasty-attack",
    scopeOf("skirmish", 180, { fires: "simulated", logistics: "abstracted", drones: "abstracted" }),
    [
      unit("b-ifv-1", "blue", "generic/ifv", 3, -500, 0, "1 Platoon"),
      unit("b-ifv-2", "blue", "generic/ifv", 3, 0, 0, "2 Platoon"),
      unit("b-ifv-3", "blue", "generic/ifv", 3, 500, 0, "3 Platoon"),
      unit("b-tanks", "blue", "generic/mbt", 4, 0, -400, "Tank platoon"),
      unit("b-mortar", "blue", "generic/mortar120", 2, 0, -1800, "Mortar section"),
      unit("b-sappers", "blue", "generic/engineerSection", 2, 300, -700, "Engineer section"),
      unit("r-inf-1", "red", "generic/infantrySection", 3, -300, 3200, "Red 1 Section", "regular"),
      unit("r-inf-2", "red", "generic/infantrySection", 3, 300, 3300, "Red 2 Section", "regular"),
      unit("r-atgm", "red", "generic/atgmTeam", 2, 0, 3600, "Red ATGM team"),
      unit("r-ifv", "red", "generic/ifv", 3, 0, 3800, "Red IFV platoon"),
      unit("r-mortar", "red", "generic/mortar120", 2, 0, 5500, "Red mortars"),
    ],
    { obstacles: [obstacle("red", "minefield", 0, 2600, 1800, 250, 0.5)], environment: { startHour: 5.5 } },
  );
}

/** Every milestone scenario, by milestone. */
export const PRESET_SCENARIOS: Record<string, () => RealtimeScenario> = {
  M2: mechCompanyAttack,
};

export const presetHelpers = { unit, entity, obstacle, scopeOf, scenario };
export type { HqSpec };
