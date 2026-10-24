// ── realtime/world/testkit.ts ────────────────────────────────────────────
// Games built for the world tests and the vignettes (W12): forces placed by
// catalogue id or force-list key at metre offsets, entities, obstacles,
// weather and a scope, on flat or generated ground, with the rules deciding.
// Not a test file itself, so the headless batch runner can use it too.

import { offsetToLatLng, toGameStateFromPlaced, DEFAULT_ORIGIN, type PlacedElement } from "../../lib/forceBuilder";
import { flatTerrain } from "../../lib/lineOfSight";
import { proceduralTerrain, STANDARD_GROUND } from "../../lib/proceduralTerrain";
import type { LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import type { TroopQualityName } from "../../rules/forceList";
import { createRng } from "../../rules/dice";
import { HOUSE_V1 } from "../../rules/ruleset";
import { createRealtimeState } from "../engine/engine";
import { DEFAULT_TIMING } from "../engine/timing";
import type { RtConfig, RtOrder, RtState } from "../engine/types";
import { DOMAINS, scopeFor, type Domain, type DomainMode, type Scope } from "../scope";
import { createWorld, type EntitySpec, type HqSpec, type ObstacleSpec, type WorldSetup } from "./create";
import type { ControlMeasure, CyberAccess, Environment } from "./types";
import { InThreadRunner } from "../engine/runnerHandle";
import type { WorldDecider } from "./deciders";

export interface KitUnit {
  id: string;
  side: Side;
  platform: string;
  count?: number;
  quality?: TroopQualityName;
  /** Metres east and north of the origin. */
  east: number;
  north: number;
  label?: string;
  commandRating?: number;
  order?: RtOrder;
}

export interface KitEntity extends Omit<EntitySpec, "at"> {
  east: number;
  north: number;
}

export interface KitOptions {
  units: KitUnit[];
  entities?: KitEntity[];
  obstacles?: (Omit<ObstacleSpec, "at"> & { east: number; north: number })[];
  hqs?: HqSpec[];
  measures?: ControlMeasure[];
  cyber?: WorldSetup["cyber"];
  victory?: WorldSetup["victory"];
  events?: WorldSetup["events"];
  nais?: WorldSetup["nais"];
  env?: Partial<Environment>;
  /** Domains to set, on top of a skirmish's (land simulated, the rest off). */
  domains?: Partial<Record<Domain, DomainMode>>;
  scope?: Partial<Scope>;
  seed?: string;
  ground?: "flat" | "generated";
  comms?: RtConfig["comms"];
  maxDurationS?: number;
  breakpoint?: number;
  origin?: LatLng;
}

export const at = (east: number, north: number, origin: LatLng = DEFAULT_ORIGIN): LatLng => offsetToLatLng(origin, east, north);

/** A game: its state at the start, with a world, and its config. */
export function kitGame(o: KitOptions): { state: RtState; config: RtConfig } {
  const origin = o.origin ?? DEFAULT_ORIGIN;
  const base = scopeFor("custom");
  const domains = Object.fromEntries(DOMAINS.map((d) => [d, d === "land" ? "simulated" : "off"])) as Record<Domain, DomainMode>;
  const scope: Scope = { ...base, ...o.scope, domains: { ...domains, ...o.scope?.domains, ...o.domains } };
  const config: RtConfig = {
    ruleset: HOUSE_V1,
    terrain: o.ground === "generated" ? proceduralTerrain({ ...STANDARD_GROUND, seed: o.seed ?? "kit", origin }) : flatTerrain(),
    rng: createRng(o.seed ?? "kit"),
    timing: { ...DEFAULT_TIMING, maxDurationS: o.maxDurationS ?? 3600, breakpoint: o.breakpoint ?? 0.9 },
    comms: o.comms ?? "perfect",
    scope,
  };
  const placed: PlacedElement[] = o.units.map((u) => ({
    id: u.id,
    label: u.label ?? u.id,
    side: u.side,
    platform: u.platform,
    platformCount: u.count ?? 2,
    troopQuality: u.quality ?? "regular",
    position: at(u.east, u.north, origin),
    ...(u.commandRating ? { commandRating: u.commandRating } : {}),
  }));
  let state = createRealtimeState(toGameStateFromPlaced(placed, HOUSE_V1, { gameId: o.seed ?? "kit" }));
  for (const u of o.units) {
    if (u.order) state = { ...state, units: { ...state.units, [u.id]: { ...state.units[u.id], order: u.order } } };
  }
  const setup: WorldSetup = {
    platforms: Object.fromEntries(o.units.map((u) => [u.id, u.platform])),
    entities: (o.entities ?? []).map(({ east, north, ...e }) => ({ ...e, at: at(east, north, origin) })),
    obstacles: (o.obstacles ?? []).map(({ east, north, ...ob }) => ({ ...ob, at: at(east, north, origin) })),
    ...(o.hqs ? { hqs: o.hqs } : {}),
    ...(o.measures ? { measures: o.measures } : {}),
    ...(o.cyber ? { cyber: o.cyber as CyberAccess[] } : {}),
    ...(o.victory ? { victory: o.victory } : {}),
    ...(o.events ? { events: o.events } : {}),
    ...(o.nais ? { nais: o.nais } : {}),
    ...(o.env ? { environment: o.env } : {}),
  };
  state = { ...state, world: createWorld(state, config, setup) };
  return { state, config };
}

/** Run a kit game for `seconds` with the rules deciding (or the given world deciders). */
export async function runKit(
  game: { state: RtState; config: RtConfig },
  seconds: number,
  options: { worldDeciders?: Partial<Record<Side, WorldDecider>> } = {},
): Promise<InThreadRunner> {
  const runner = new InThreadRunner(game.state, game.config, { ...(options.worldDeciders ? { worldDeciders: options.worldDeciders } : {}), logLimit: 5000 });
  await runner.advance(seconds);
  return runner;
}
