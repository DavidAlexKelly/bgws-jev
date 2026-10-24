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

/**
 * M6 — Sustained fight: a three-day division offensive against a defending
 * brigade, each side fed by a railhead and a depot through convoys on the
 * board, wounded evacuated by ambulance to treatment (W10). Formations far
 * from the enemy move aggregated and come apart as they close (§4.2).
 */
export function sustainedOffensive(): RealtimeScenario {
  const forces: PlacedElement[] = [unit("b-div", "blue", "generic/cp-division", 8, 0, -32_000, "Division main HQ", "regular", 4)];
  const hqs: HqSpec[] = [{ id: "blue-div", side: "blue", echelon: "division", postId: "b-div", subordinates: [], planner: "land", seat: "heuristic" }];
  ["1", "2"].forEach((bde, b) => {
    const east = (b - 0.5) * 12_000;
    const bdeId = `blue-bde-${bde}`;
    forces.push(unit(`b-bde-${bde}`, "blue", "generic/cp-brigade", 5, east, -26_000, `${bde} Brigade HQ`, "regular", 3));
    hqs.push({ id: bdeId, side: "blue", echelon: "brigade", postId: `b-bde-${bde}`, parent: "blue-div", subordinates: [], planner: "land", seat: "heuristic", alternate: "blue-div" });
    hqs[0].subordinates.push(bdeId);
    ["a", "b"].forEach((bn, n) => {
      const x = east + (n - 0.5) * 5000;
      const bnId = `blue-${bde}${bn}`;
      forces.push(unit(`b-${bde}${bn}-hq`, "blue", "generic/cp-battalion", 3, x, -22_000, `${bde}${bn.toUpperCase()} Battalion HQ`, "regular", 3));
      forces.push(unit(`b-${bde}${bn}-1`, "blue", "generic/mbt", 4, x - 800, -20_000, `${bde}${bn.toUpperCase()} tanks`));
      forces.push(unit(`b-${bde}${bn}-2`, "blue", "generic/ifv", 3, x + 800, -20_000, `${bde}${bn.toUpperCase()} mech`));
      forces.push(unit(`b-${bde}${bn}-3`, "blue", "generic/ifv", 3, x, -21_000, `${bde}${bn.toUpperCase()} mech reserve`));
      hqs.push({ id: bnId, side: "blue", echelon: "battalion", postId: `b-${bde}${bn}-hq`, parent: bdeId, subordinates: [`b-${bde}${bn}-1`, `b-${bde}${bn}-2`, `b-${bde}${bn}-3`], planner: "land", seat: "heuristic", alternate: bdeId });
      hqs.find((h) => h.id === bdeId)!.subordinates.push(bnId);
    });
    forces.push(unit(`b-${bde}-guns`, "blue", "generic/sph155", 3, east, -30_000, `${bde} Brigade artillery`));
    forces.push(unit(`b-${bde}-shorad`, "blue", "generic/shorad", 2, east, -24_000, `${bde} Brigade SHORAD`));
    forces.push(unit(`b-${bde}-isr`, "blue", "generic/droneIsrTeam", 1, east, -19_000, `${bde} Brigade ISR`));
    hqs.find((h) => h.id === bdeId)!.subordinates.push(`b-${bde}-guns`, `b-${bde}-shorad`, `b-${bde}-isr`);
  });
  forces.push(unit("b-convoy-1", "blue", "generic/convoyTruck", 4, -1000, -44_000, "Divisional transport 1"));
  forces.push(unit("b-convoy-2", "blue", "generic/convoyTruck", 4, 1000, -44_000, "Divisional transport 2"));
  forces.push(unit("b-amb-1", "blue", "generic/medevac", 2, 0, -43_000, "Divisional ambulances"));
  // The defending brigade, in depth.
  forces.push(unit("r-bde", "red", "generic/cp-brigade", 5, 0, 30_000, "Red brigade HQ", "regular", 3));
  const red: HqSpec = { id: "red-bde", side: "red", echelon: "brigade", postId: "r-bde", subordinates: [], planner: "land", seat: "heuristic" };
  hqs.push(red);
  ["x", "y"].forEach((bn, n) => {
    const x = (n - 0.5) * 14_000;
    const bnId = `red-${bn}`;
    forces.push(unit(`r-${bn}-hq`, "red", "generic/cp-battalion", 3, x, 16_000, `Red ${bn.toUpperCase()} Battalion HQ`, "regular", 3));
    const subs = [`r-${bn}-1`, `r-${bn}-2`, `r-${bn}-at`, `r-${bn}-tanks`];
    forces.push(unit(subs[0], "red", "generic/infantrySection", 3, x - 1500, 8000, `Red ${bn.toUpperCase()}1 platoon`));
    forces.push(unit(subs[1], "red", "generic/infantrySection", 3, x + 1500, 8000, `Red ${bn.toUpperCase()}2 platoon`));
    forces.push(unit(subs[2], "red", "generic/atgmTeam", 2, x, 9000, `Red ${bn.toUpperCase()} ATGM`));
    forces.push(unit(subs[3], "red", "generic/mbt", 4, x, 13_000, `Red ${bn.toUpperCase()} counter-attack`));
    hqs.push({ id: bnId, side: "red", echelon: "battalion", postId: `r-${bn}-hq`, parent: "red-bde", subordinates: subs, planner: "land", seat: "heuristic", alternate: "red-bde" });
    red.subordinates.push(bnId);
  });
  forces.push(unit("r-guns", "red", "generic/sph155", 3, 0, 24_000, "Red artillery"));
  forces.push(unit("r-fpv", "red", "generic/fpvTeamRf", 1, 0, 12_000, "Red FPV team"));
  forces.push(unit("r-convoy", "red", "generic/convoyTruck", 4, 0, 46_000, "Red transport"));
  forces.push(unit("r-amb", "red", "generic/medevac", 2, 0, 45_000, "Red ambulances"));
  red.subordinates.push("r-guns", "r-fpv");
  return scenario(
    "M6 — Sustained offensive",
    "m6-sustained",
    scopeOf("sustainedOffensive", 3 * 24 * 60, { land: "simulated", drones: "simulated", ew: "simulated", fires: "simulated", airDefence: "simulated", logistics: "simulated", medical: "simulated", air: "abstracted", space: "abstracted", cyber: "off" }, { area: { widthKm: 60, depthKm: 200 }, echelons: { highest: "division", lowest: "platoon" } }),
    forces,
    {
      hqs,
      entities: [
        entity("b-depot", "blue", "generic/depot", 0, -45_000, { label: "Blue divisional depot" }),
        entity("b-rail", "blue", "generic/railhead", 0, -130_000, { label: "Blue railhead" }),
        entity("r-depot", "red", "generic/depot", 0, 48_000, { label: "Red brigade depot" }),
        entity("r-rail", "red", "generic/railhead", 0, 140_000, { label: "Red railhead" }),
      ],
      obstacles: [obstacle("red", "minefield", -7000, 6000, 5000, 300, 0.4), obstacle("red", "minefield", 7000, 6000, 5000, 300, 0.4)],
      environment: { startHour: 5 },
    },
  );
}

/**
 * M7 — Littoral: a task group holding a strait open against a coastal enemy
 * (Red Sea), which throws one-way drones, anti-ship missiles cued by a drone
 * overhead, and explosive USVs at it, while neutral merchants try to pass.
 * The group's cells are refilled only in port, 150 km back (W8).
 */
export function littoral(): RealtimeScenario {
  const forces: PlacedElement[] = [
    unit("r-asm-1", "red", "generic/coastalAsm", 2, 150_000, -10_000, "Red coastal battery north"),
    unit("r-asm-2", "red", "generic/coastalAsm", 2, 150_000, 25_000, "Red coastal battery south"),
    unit("r-hq", "red", "generic/cp-brigade", 5, 170_000, 0, "Red coastal command", "regular", 3),
  ];
  const raid: EntitySpec[] = [];
  for (let i = 0; i < 24; i++) {
    const decoy = i % 4 === 3;
    raid.push(entity(`r-owa-${i + 1}`, "red", decoy ? "generic/owaDecoy" : "generic/owaDrone", 145_000, -30_000 + i * 2500, { altM: 500, label: decoy ? `Decoy ${i + 1}` : `Drone ${i + 1}`, task: { kind: "strike", targetId: i % 3 === 0 ? "b-ffg" : "b-ddg", aimAt: p(0, 0), altM: 500 } }));
  }
  for (let i = 0; i < 4; i++) raid.push(entity(`r-usv-${i + 1}`, "red", "generic/usv", 120_000, -6000 + i * 4000, { label: `USV ${i + 1}`, task: { kind: "strike", targetId: "b-ffg", aimAt: p(5000, 3000) } }));
  // The strait: sea between two coasts, 260 km across.
  const sea = { waters: [[p(-400_000, -200_000), p(130_000, -200_000), p(130_000, 200_000), p(-400_000, 200_000)]] };
  return scenario(
    "M7 — Littoral: holding the strait",
    "m7-littoral",
    scopeOf("maritime", 24 * 60, { naval: "simulated", air: "simulated", airDefence: "simulated", drones: "simulated", ew: "simulated", land: "simulated", space: "abstracted", logistics: "abstracted", cyber: "off" }, { area: { widthKm: 600, depthKm: 400 }, echelons: { highest: "component", lowest: "platoon" }, planners: ["overall", "maritime", "air"] }),
    forces,
    {
      entities: [
        entity("b-ddg", "blue", "generic/destroyer", 0, 0, { label: "Blue destroyer", parent: "b-port", task: { kind: "station", at: p(0, 0), radiusM: 10_000 } }),
        entity("b-ffg", "blue", "generic/frigate", 5000, 3000, { label: "Blue frigate", parent: "b-port", task: { kind: "station", at: p(5000, 3000), radiusM: 10_000 } }),
        entity("b-port", "blue", "generic/port", -150_000, 0, { label: "Blue naval base" }),
        entity("r-uav", "red", "generic/male", 30_000, 0, { altM: 5000, label: "Red surveillance drone", task: { kind: "orbit", at: p(20_000, 0), radiusM: 5000, altM: 5000 } }),
        entity("m-1", "neutral", "generic/merchant", -120_000, 30_000, { label: "Bulk carrier", lane: [p(-120_000, 30_000), p(100_000, 30_000)] }),
        entity("m-2", "neutral", "generic/merchant", 100_000, -30_000, { label: "Container ship", lane: [p(100_000, -30_000), p(-120_000, -30_000)] }),
        ...raid,
      ],
      obstacles: [{ owner: "red", kind: "seaMines", at: p(110_000, 60_000), widthM: 20_000, depthM: 10_000, bearingDeg: 0, density: 0.6 }],
      sea,
      environment: { startHour: 6, seaState: 2 },
      victory: [{ kind: "through", side: "blue", ids: ["m-1", "m-2"], at: p(100_000, 30_000), radiusM: 5000, count: 1, label: "Shipping through the strait" }],
    },
  );
}

/**
 * M8 — Joint strike: the opening of a Rising Lion-like operation [ref R6.5].
 * Loitering drones staged close to the enemy's air-defence radars hit them
 * first; a cyber access blinds the shared air picture; SEAD flights open a
 * corridor on the radars SIGINT has fixed; then strikes follow on what the
 * kill web nominates — from the air, from a submarine's cruise missiles, from
 * imagery of the depots behind. Blue wins by taking down the defences and
 * the command that runs them (W9, W11).
 */
export function jointStrike(): RealtimeScenario {
  const forces: PlacedElement[] = [
    unit("b-jfc", "blue", "generic/cp-division", 8, 0, -120_000, "Joint force HQ", "veteran", 5),
    unit("b-loiter-1", "blue", "generic/loiterTeam", 1, -8000, 52_000, "Staged drone team west"),
    unit("b-loiter-2", "blue", "generic/loiterTeam", 1, 9000, 48_000, "Staged drone team east"),
    unit("b-esm-1", "blue", "generic/esmTeam", 1, -25_000, 20_000, "SIGINT west"),
    unit("b-esm-2", "blue", "generic/esmTeam", 1, 25_000, 20_000, "SIGINT east"),
    unit("r-ad-hq", "red", "generic/cp-brigade", 5, 0, 110_000, "Red air-defence command", "regular", 3),
    unit("r-sam-long", "red", "generic/samLong", 2, 0, 90_000, "Red long-range SAM"),
    unit("r-sam-1", "red", "generic/samMedium", 2, -10_000, 60_000, "Red SAM west"),
    unit("r-sam-2", "red", "generic/samMedium", 2, 12_000, 55_000, "Red SAM east"),
    unit("r-shorad", "red", "generic/shorad", 2, 0, 105_000, "Red point defence"),
  ];
  const hqs: HqSpec[] = [
    { id: "blue-jfc", side: "blue", echelon: "joint", postId: "b-jfc", subordinates: ["b-loiter-1", "b-loiter-2", "b-esm-1", "b-esm-2"], planner: "overall", seat: "heuristic" },
    { id: "red-ad", side: "red", echelon: "brigade", postId: "r-ad-hq", subordinates: ["r-sam-long", "r-sam-1", "r-sam-2", "r-shorad"], planner: "air", seat: "heuristic" },
  ];
  return scenario(
    "M8 — Joint strike",
    "m8-joint",
    scopeOf("strike", 24 * 60, { land: "simulated", air: "simulated", airDefence: "simulated", space: "simulated", ew: "simulated", cyber: "simulated", drones: "simulated", naval: "simulated", fires: "off", logistics: "abstracted" }, { area: { widthKm: 400, depthKm: 400 }, planners: ["overall", "air", "maritime", "spaceCyber"] }),
    forces,
    {
      hqs,
      entities: [
        entity("b-base", "blue", "generic/airbase", 0, -150_000, { label: "Blue airbase" }),
        entity("b-ssn", "blue", "generic/ssn", 0, -220_000, { altM: -100, label: "Blue submarine" }),
        entity("b-sar", "blue", "generic/satSar", 0, 0, { label: "Blue radar satellite" }),
        entity("b-eo", "blue", "generic/satEo", 0, 0, { label: "Commercial imaging satellite", provider: "commercial" }),
        entity("b-mw", "blue", "generic/missileWarning", 0, 0, { label: "Blue missile warning" }),
        entity("b-cyber", "blue", "generic/cyberCell", 0, -200_000, { label: "Blue cyber cell" }),
        entity("r-depot", "red", "generic/depot", 20_000, 140_000, { label: "Red missile depot" }),
        entity("r-base", "red", "generic/airbase", -30_000, 180_000, { label: "Red airbase" }),
      ],
      sea: { waters: [[p(-300_000, -300_000), p(300_000, -300_000), p(300_000, -180_000), p(-300_000, -180_000)]] },
      cyber: [{ id: "access-ad", side: "blue", target: "airDefence", system: "red air-defence network", readiness: 0, pEffect: 0.7, durationS: 6 * 3600, detectionRisk: 0.3, window: { from: 0, until: 2 * 3600 } }],
      nais: [
        { side: "blue", label: "NAI 1: the air-defence command", at: p(0, 108_000), priority: 5 },
        { side: "blue", label: "NAI 2: the missile depot", at: p(18_000, 140_000), priority: 4 },
      ],
      network: { blue: "datalink", red: "voice" },
      environment: { startHour: 2, cloudCover: 0.3 },
      victory: [{ kind: "targetSet", side: "blue", targets: ["r-sam-long", "r-sam-1", "r-sam-2", "r-ad-hq"], share: 0.75, label: "The air defences and their command down" }],
    },
  );
}

/** Every milestone scenario, by milestone. */
export const PRESET_SCENARIOS: Record<string, () => RealtimeScenario> = {
  M2: mechCompanyAttack,
  M3: battalionAttack,
  M4: droneLine,
  M5: airOverTheBattle,
  M6: sustainedOffensive,
  M7: littoral,
  M8: jointStrike,
};

export const presetHelpers = { unit, entity, obstacle, scopeOf, scenario };
export type { HqSpec };
