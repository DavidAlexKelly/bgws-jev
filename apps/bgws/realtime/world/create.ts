// ── realtime/world/create.ts ─────────────────────────────────────────────
// Setting a game's world up: the components of every land unit from its
// archetype, the entities placed in the scenario (ships, airbases, depots,
// satellites, command posts), obstacles, weather and the command tree.
//
// A game gets a world only when something needs one: a domain other than
// land is simulated or abstracted, or the scenario places world content.
// Otherwise `RtState.world` stays absent and the game is exactly as before.

import type { LatLng } from "../../lib/board";
import { SIDES, type Side } from "../../lib/state";
import { hasRecord, recordFor } from "../catalogue";
import type { CatalogueRecord } from "../catalogue/schema";
import type { RtConfig, RtState } from "../engine/types";
import { archetypeFor } from "../model/archetypes";
import { DOMAINS, changesEnvironment, type Scope } from "../scope";
import { DEFAULT_ENVIRONMENT, scopeEnvironment } from "./environment";
import type {
  ControlMeasure,
  CyberAccess,
  Entity,
  EntityTask,
  Environment,
  LandComponents,
  MountState,
  Obstacle,
  Owner,
  NamedArea,
  ScenarioEvent,
  SeaArea,
  Stores,
  VictoryCondition,
  WorldState,
} from "./types";

/** An entity as a scenario places it. */
export interface EntitySpec {
  id: string;
  owner: Owner;
  archetype: string;
  label?: string;
  at: LatLng;
  altM?: number;
  task?: EntityTask;
  parent?: string;
  /** Satellites: where in the orbit at t = 0, degrees. */
  orbitPhaseDeg?: number;
  orbitRaanDeg?: number;
  provider?: "military" | "commercial";
  /** Neutral shipping: its lane. */
  lane?: LatLng[];
  template?: string;
  status?: Entity["status"];
}

export interface ObstacleSpec {
  id?: string;
  owner: Side;
  kind: Obstacle["kind"];
  at: LatLng;
  widthM: number;
  depthM: number;
  bearingDeg?: number;
  density?: number;
}

/** One HQ in the command tree, as the scenario gives it (W4). */
export interface HqSpec {
  id: string;
  side: Side;
  echelon: Scope["echelons"]["highest"];
  /** The land unit or entity it sits in. */
  postId: string;
  parent?: string;
  subordinates: string[];
  alternate?: string;
  planner?: Scope["planners"][number];
  seat?: "claude" | "heuristic" | "human";
}

export interface WorldSetup {
  /** Land unit id → the platform it was placed as (a force list key or a catalogue id). */
  platforms: Record<string, string>;
  entities?: EntitySpec[];
  obstacles?: ObstacleSpec[];
  environment?: Partial<Environment>;
  hqs?: HqSpec[];
  measures?: ControlMeasure[];
  cyber?: Omit<CyberAccess, "executedAt" | "effectUntil" | "detected">[];
  /** Victory conditions the world judges (world/victory.ts). */
  victory?: VictoryCondition[];
  /** Water areas (W8): polygons of sea; with none, all is open ocean. */
  sea?: SeaArea;
  /** Named areas of interest for each side's collection (W9). */
  nais?: NamedArea[];
  /** Scenario events at set times (W9): providers cutting service, counterspace. */
  events?: ScenarioEvent[];
  /** Each side's kill-web network (W11): data link (flat) or voice (hierarchical). Data link by default. */
  network?: Partial<Record<Side, "voice" | "datalink">>;
}

/** Does this game need a world? */
export function needsWorld(scope: Scope | undefined, setup?: Partial<WorldSetup>): boolean {
  if (setup && ((setup.entities?.length ?? 0) > 0 || (setup.obstacles?.length ?? 0) > 0 || (setup.hqs?.length ?? 0) > 0 || (setup.victory?.length ?? 0) > 0 || (setup.events?.length ?? 0) > 0 || setup.environment)) return true;
  if (!scope) return false;
  if (changesEnvironment(scope)) return true;
  return DOMAINS.some((d) => d !== "land" && scope.domains[d] !== "off");
}

export function storesOf(record: CatalogueRecord, count = record.body.count): Stores {
  const s = record.stores;
  return {
    fuelL: s.fuelL * (record.domain === "land" ? count : 1),
    ammo: { ...s.ammo },
    batteryWh: s.batteryWh,
    medical: s.medical,
    spares: s.spares,
  };
}

/** The world weapons a record mounts: everything the land engine does not fire itself. */
export function mountsOf(record: CatalogueRecord): MountState[] {
  return record.weapons
    .filter((w) => w.kind !== "directFire" && w.kind !== "atgm")
    .filter((w) => !["fpv", "loitering", "owa"].includes(w.kind) || record.domain !== "land")
    // A ship's or submarine's cells and tubes hold its whole load: nothing is reloaded at sea (W8).
    .map((w) => ({ weaponId: w.id, rounds: cellLoad(record, w), readyAt: 0 }));
}

/** Rounds a mount holds: a launcher's magazine on land, every cell's worth at sea. */
export function cellLoad(record: CatalogueRecord, w: CatalogueRecord["weapons"][number]): number {
  const stores = record.stores.ammo[w.family];
  if (record.domain === "surface" || record.domain === "subsurface") return stores ?? w.magazine;
  return Math.min(w.magazine, stores ?? w.magazine);
}

const NO_DAMAGE = { mobility: false, firepower: false, sensor: false, comms: false };

export function landComponents(archetype: string): LandComponents {
  const record = recordFor(hasRecord(archetype) ? archetype : "generic/infantrySection");
  return {
    archetype: record.id,
    stores: storesOf(record),
    mounts: mountsOf(record),
    drones: record.carried.drones?.count ?? 0,
    dismounts: { carried: record.carried.dismounts?.sections ?? 0, out: [] },
    emcon: "free",
    weaponControl: "tight",
    onAirS: 0,
    dugIn: 0,
    positionErrorM: 10,
    stocksLowAt: null,
    wounded: [],
    killed: 0,
    disabled: 0,
    lastHqContactAt: 0,
    damage: { ...NO_DAMAGE },
  };
}

/** An entity from its archetype, ready to place. */
export function makeEntity(spec: EntitySpec): Entity {
  const record = recordFor(spec.archetype);
  const isAir = record.domain === "air";
  const endurance = record.mover.enduranceH;
  return {
    id: spec.id,
    owner: spec.owner,
    archetype: record.id,
    label: spec.label ?? record.name,
    domain: record.domain,
    pos: spec.at,
    altM: spec.altM ?? (record.domain === "subsurface" ? -50 : record.domain === "space" ? (record.mover.orbitAltKm ?? 500) * 1000 : 0),
    headingDeg: 0,
    speedMs: 0,
    status: spec.status ?? (isAir && spec.parent ? "landed" : "active"),
    fit: record.body.count,
    hp: record.body.stayingPower * (record.domain === "surface" || record.domain === "subsurface" || record.domain === "node" ? 1 : record.body.count),
    damage: { ...NO_DAMAGE },
    task: spec.task ?? { kind: "idle" },
    stores: storesOf(record),
    mounts: mountsOf(record).length ? mountsOf(record) : record.weapons.map((w) => ({ weaponId: w.id, rounds: w.magazine, readyAt: 0 })),
    ...(endurance != null ? { enduranceS: endurance * 3600 } : {}),
    ...(spec.parent ? { parent: spec.parent } : {}),
    template: spec.template ?? record.template,
    emcon: "free",
    weaponControl: "tight",
    ...(spec.orbitPhaseDeg != null ? { orbitPhaseDeg: spec.orbitPhaseDeg } : {}),
    ...(spec.orbitRaanDeg != null ? { orbitRaanDeg: spec.orbitRaanDeg } : {}),
    ...(spec.provider ? { provider: spec.provider } : {}),
    costUsd: record.costUsd,
    ...(record.plannedBy ? { plannedBy: record.plannedBy } : {}),
    lastFiredAt: -Infinity,
    ...(isAir ? { bingo: 0.3 } : {}),
    ...(spec.lane ? { lane: spec.lane } : {}),
    history: [],
  };
}

export function emptyWorld(env: Partial<Environment> = {}): WorldState {
  return {
    version: 1,
    env: { ...DEFAULT_ENVIRONMENT, ...env },
    land: {},
    entities: {},
    tracks: { blue: {}, red: {} },
    obstacles: [],
    breaches: [],
    missions: [],
    emissions: [],
    fixes: { blue: {}, red: {} },
    jammers: [],
    hqs: {},
    messages: [],
    measures: [],
    chains: [],
    pnt: { blue: { global: 1 }, red: { global: 1 } },
    cyber: [],
    supply: [],
    questions: [],
    queue: [],
    stats: {
      spentUsd: { blue: 0, red: 0 },
      lostUsd: { blue: 0, red: 0 },
      interceptsTried: 0,
      interceptsMade: 0,
      munitions: {},
      landLossDepthKm: [],
      evacuationS: [],
      breachS: [],
      reportLatencyS: { blue: [], red: [] },
      reportLatencyByHq: {},
    },
    startLines: { blue: { lat: 0, lng: 0 }, red: { lat: 0, lng: 0 } },
    outages: {},
    support: [],
    orderQueue: [],
    plans: [],
    structured: {},
    joinedStrength: { blue: 0, red: 0 },
    memo: {},
    nextId: 1,
    log: [],
  };
}

/** Hooks other modules add to world creation (command tree, support services). */
type CreateHook = (world: WorldState, state: RtState, config: RtConfig, setup: WorldSetup) => void;
const HOOKS: { order: number; hook: CreateHook }[] = [];
export function onCreateWorld(order: number, hook: CreateHook): void {
  HOOKS.push({ order, hook });
  HOOKS.sort((a, b) => a.order - b.order);
}

/** The world for a game about to start. */
export function createWorld(state: RtState, config: RtConfig, setup: WorldSetup): WorldState {
  // The scope's time dimension sets the light and weather; the scenario's own environment, if any, wins.
  const world = emptyWorld({ ...scopeEnvironment(config.scope), ...setup.environment });
  for (const fe of Object.values(state.game.forceElements)) {
    const platform = setup.platforms[fe.id] ?? "";
    world.land[fe.id] = landComponents(archetypeFor(platform, { commandRating: fe.commandRating }));
  }
  for (const side of SIDES) {
    const own = Object.values(state.game.forceElements).filter((fe) => fe.side === side);
    if (own.length) {
      world.startLines[side] = {
        lat: own.reduce((s, fe) => s + fe.position.lat, 0) / own.length,
        lng: own.reduce((s, fe) => s + fe.position.lng, 0) / own.length,
      };
    }
  }
  for (const spec of setup.entities ?? []) world.entities[spec.id] = makeEntity(spec);
  world.obstacles = (setup.obstacles ?? []).map((o, i) => ({
    id: o.id ?? `obstacle-${i + 1}`,
    owner: o.owner,
    kind: o.kind,
    at: o.at,
    widthM: o.widthM,
    depthM: o.depthM,
    bearingDeg: o.bearingDeg ?? 0,
    density: o.density ?? (o.kind === "minefield" ? 0.5 : 0),
    lanes: [],
    knownTo: [o.owner],
  }));
  world.measures = setup.measures ?? [];
  if (setup.victory?.length) world.victory = setup.victory;
  if (setup.sea?.waters.length) world.sea = setup.sea;
  if (setup.nais?.length) world.nais = setup.nais;
  if (setup.network) world.network = { blue: setup.network.blue ?? "datalink", red: setup.network.red ?? "datalink" };
  // Scenario events go on the scheduler's queue, in order.
  for (const [i, e] of (setup.events ?? []).entries()) world.queue.push({ at: e.at, kind: "scenarioEvent", key: String(i), data: { kind: e.kind, side: e.side, ...(e.durationS != null ? { durationS: e.durationS } : {}), ...(e.target ? { target: e.target } : {}), ...(e.quality != null ? { quality: e.quality } : {}) } });
  world.queue.sort((a, b) => a.at - b.at || a.kind.localeCompare(b.kind) || a.key.localeCompare(b.key));
  world.cyber = (setup.cyber ?? []).map((a) => ({ ...a }));
  for (const { hook } of HOOKS) hook(world, state, config, setup);
  return world;
}

/** The state with its world, if it needs one. */
export function withWorld(state: RtState, config: RtConfig, setup: WorldSetup): RtState {
  if (!needsWorld(config.scope, setup)) return state;
  return { ...state, world: createWorld(state, config, setup) };
}
