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

/**
 * M3 — Battalion C2: a battalion attack. Three company HQs and a battalion
 * HQ, each on its own nets, with terrain-limiting VHF; planners on their
 * battle rhythm and on CCIR; air and space abstracted (W4).
 */
export function battalionAttack(): RealtimeScenario {
  const forces: PlacedElement[] = [unit("b-bn", "blue", "generic/cp-battalion", 3, 0, -8000, "Battalion HQ", "regular", 3)];
  const hqs: HqSpec[] = [{ id: "blue-bn", side: "blue", echelon: "battalion", postId: "b-bn", subordinates: ["blue-a", "blue-b", "blue-c"], planner: "land", seat: "claude" }];
  ["a", "b", "c"].forEach((coy, i) => {
    const east = (i - 1) * 2500;
    forces.push(unit(`b-${coy}-hq`, "blue", "generic/cp-company", 2, east, -2800, `${coy.toUpperCase()} Company HQ`, "regular", 2));
    forces.push(unit(`b-${coy}-1`, "blue", "generic/mbt", 4, east - 400, -2000, `${coy.toUpperCase()}1 tanks`));
    forces.push(unit(`b-${coy}-2`, "blue", "generic/ifv", 3, east + 400, -2000, `${coy.toUpperCase()}2 mech`));
    hqs.push({ id: `blue-${coy}`, side: "blue", echelon: "company", postId: `b-${coy}-hq`, parent: "blue-bn", subordinates: [`b-${coy}-1`, `b-${coy}-2`], planner: "land", seat: "heuristic", alternate: "blue-bn" });
  });
  forces.push(unit("b-mortar", "blue", "generic/mortar120", 2, 0, -4500, "Battalion mortars"));
  hqs[0].subordinates.push("b-mortar");
  forces.push(unit("r-hq", "red", "generic/cp-company", 2, 0, 4500, "Red company HQ", "regular", 2));
  const redSubs: string[] = [];
  for (let i = 0; i < 3; i++) {
    forces.push(unit(`r-${i}`, "red", "generic/infantrySection", 3, (i - 1) * 2200, 2000, `Red ${i + 1} platoon`));
    forces.push(unit(`r-at-${i}`, "red", "generic/atgmTeam", 2, (i - 1) * 2200 + 300, 2400, `Red ATGM ${i + 1}`));
    redSubs.push(`r-${i}`, `r-at-${i}`);
  }
  forces.push(unit("r-tanks", "red", "generic/mbt", 4, 0, 3800, "Red counter-attack force"));
  forces.push(unit("r-mortar", "red", "generic/mortar120", 2, 0, 5500, "Red mortars"));
  redSubs.push("r-tanks", "r-mortar");
  hqs.push({ id: "red-hq", side: "red", echelon: "company", postId: "r-hq", subordinates: redSubs, planner: "land", seat: "heuristic" });
  return scenario(
    "M3 — Battalion attack",
    "m3-battalion",
    scopeOf("combinedArms", 240, { drones: "abstracted", ew: "off", fires: "simulated", logistics: "abstracted", air: "abstracted", space: "abstracted" }, { echelons: { highest: "battalion", lowest: "platoon" }, planners: ["overall", "land"], area: { widthKm: 20, depthKm: 20 } }),
    forces,
    { hqs, obstacles: [obstacle("red", "minefield", 0, 1200, 6000, 200, 0.4)], environment: { startHour: 4.5 } },
  );
}

/**
 * M4 — Transparent battlefield: a drone line. Drone teams on both sides 5–15
 * km apart, jammers, fibre FPVs, an interceptor screen, and a mechanised
 * push across the gap (W5, W6).
 */
export function droneLine(): RealtimeScenario {
  const forces: PlacedElement[] = [
    unit("b-hq", "blue", "generic/cp-company", 2, 0, -5000, "Blue company HQ", "regular", 2),
    unit("b-tanks", "blue", "generic/mbt", 4, -800, -1000, "Blue tanks"),
    unit("b-ifv", "blue", "generic/ifv", 3, 800, -1000, "Blue mech"),
    unit("b-isr", "blue", "generic/droneIsrTeam", 1, 0, -3500, "Blue ISR team"),
    unit("b-fpv", "blue", "generic/fpvTeamRf", 1, -1500, -3000, "Blue FPV team"),
    unit("b-fibre", "blue", "generic/fpvTeamFibre", 1, 1500, -3000, "Blue fibre FPV team"),
    unit("b-int", "blue", "generic/interceptorTeam", 1, 0, -2500, "Blue interceptor team"),
    unit("b-jam", "blue", "generic/ewJammer", 1, 0, -2000, "Blue EW"),
    unit("b-esm", "blue", "generic/esmTeam", 1, 3000, -4000, "Blue SIGINT"),
    unit("b-guns", "blue", "generic/sph155", 3, 0, -9000, "Blue artillery"),
    unit("r-hq", "red", "generic/cp-company", 2, 0, 11_000, "Red company HQ", "regular", 2),
    unit("r-inf-1", "red", "generic/infantrySection", 3, -1500, 6000, "Red 1 Platoon"),
    unit("r-inf-2", "red", "generic/infantrySection", 3, 1500, 6000, "Red 2 Platoon"),
    unit("r-isr", "red", "generic/droneIsrTeam", 1, -2000, 9000, "Red ISR team"),
    unit("r-fpv-1", "red", "generic/fpvTeamRf", 1, 0, 9500, "Red FPV team 1"),
    unit("r-fpv-2", "red", "generic/fpvTeamRf", 1, 2500, 9000, "Red FPV team 2"),
    unit("r-loiter", "red", "generic/loiterTeam", 1, 0, 13_000, "Red loitering munitions"),
    unit("r-jam", "red", "generic/ewJammer", 1, 0, 8000, "Red EW"),
    unit("r-shorad", "red", "generic/shorad", 2, 1000, 8500, "Red SHORAD"),
    unit("r-guns", "red", "generic/sph155", 3, 0, 15_000, "Red artillery"),
  ];
  return scenario(
    "M4 — Drone line",
    "m4-drone-line",
    scopeOf("droneLine", 360, { logistics: "abstracted", medical: "off" }, { echelons: { highest: "company", lowest: "platoon" }, planners: ["tactical"], area: { widthKm: 30, depthKm: 15 } }),
    forces,
    { obstacles: [obstacle("red", "minefield", 0, 4500, 5000, 300, 0.4)], environment: { startHour: 5, windMs: 5 } },
  );
}

/**
 * M5 — Air over the battle: a contested front with SEAD and CAS overhead,
 * and a Shahed-style raid on the blue airbase's layered defence (W7). Blue's
 * SIGINT fixes the red SAM for the SEAD flights; blue's guns share the sky
 * with its own aircraft. Blue wins by clearing the air over the front; red
 * by putting the airbase out of action.
 */
export function airOverTheBattle(): RealtimeScenario {
  const forces: PlacedElement[] = [
    // The land front.
    unit("b-hq", "blue", "generic/cp-company", 2, 0, -6000, "Blue company HQ", "regular", 2),
    unit("b-tanks", "blue", "generic/mbt", 4, -800, -1500, "Blue tanks"),
    unit("b-ifv", "blue", "generic/ifv", 3, 800, -1500, "Blue mech"),
    unit("b-guns", "blue", "generic/sph155", 3, 0, -14_000, "Blue artillery"),
    unit("b-esm-1", "blue", "generic/esmTeam", 1, -12_000, -5000, "Blue SIGINT west"),
    unit("b-esm-2", "blue", "generic/esmTeam", 1, 12_000, -5000, "Blue SIGINT east"),
    unit("r-hq", "red", "generic/cp-company", 2, 0, 12_000, "Red company HQ", "regular", 2),
    unit("r-inf-1", "red", "generic/infantrySection", 3, -1500, 5000, "Red 1 Platoon"),
    unit("r-inf-2", "red", "generic/infantrySection", 3, 1500, 5000, "Red 2 Platoon"),
    unit("r-shorad", "red", "generic/shorad", 2, 0, 8000, "Red SHORAD"),
    unit("r-sam", "red", "generic/samMedium", 2, 4000, 40_000, "Red SAM battery"),
    // The airbase's layered defence.
    unit("b-sam", "blue", "generic/samMedium", 2, 0, -52_000, "Blue SAM battery"),
    unit("b-shorad-1", "blue", "generic/shorad", 2, -2000, -58_000, "Blue SHORAD west"),
    unit("b-shorad-2", "blue", "generic/shorad", 2, 2000, -58_000, "Blue SHORAD east"),
  ];
  const raid: EntitySpec[] = [];
  for (let i = 0; i < 24; i++) {
    // About 60% armed, 40% decoys [P10], launched from far behind red's lines at the airbase.
    const decoy = i % 5 >= 3;
    raid.push(entity(`r-owa-${i + 1}`, "red", decoy ? "generic/owaDecoy" : "generic/owaDrone", -25_000 + i * 2000, 90_000 + (i % 3) * 3000, { altM: 1500, label: decoy ? `Decoy ${i + 1}` : `Shahed ${i + 1}`, task: { kind: "strike", aimAt: p((i % 3) * 60 - 60, -60_000 + (i % 2) * 60), altM: 1500 } }));
  }
  return scenario(
    "M5 — Air over the battle",
    "m5-air",
    scopeOf("airCampaign", 360, { land: "simulated", fires: "simulated", drones: "simulated", ew: "simulated", air: "simulated", airDefence: "simulated", space: "abstracted", logistics: "abstracted" }, { area: { widthKm: 100, depthKm: 200 }, echelons: { highest: "component", lowest: "platoon" } }),
    forces,
    {
      entities: [entity("b-base", "blue", "generic/airbase", 0, -60_000, { label: "Blue airbase" }), entity("r-base", "red", "generic/airbase", 0, 160_000, { label: "Red airbase" }), ...raid],
      environment: { startHour: 6 },
      victory: [
        { kind: "airSuperiority", side: "blue", at: p(0, 30_000), radiusM: 25_000, holdS: 1800, label: "Air superiority over the front" },
        { kind: "targetSet", side: "red", targets: ["b-base"], share: 1, label: "The blue airbase out of action" },
      ],
    },
  );
}

/** Every milestone scenario, by milestone. */
export const PRESET_SCENARIOS: Record<string, () => RealtimeScenario> = {
  M2: mechCompanyAttack,
  M3: battalionAttack,
  M4: droneLine,
  M5: airOverTheBattle,
};

export const presetHelpers = { unit, entity, obstacle, scopeOf, scenario };
export type { HqSpec };
