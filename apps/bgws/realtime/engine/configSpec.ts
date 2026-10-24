// ── configSpec.ts ────────────────────────────────────────────────────────
// Step 1 of docs/plan/WORKER_AND_SCHEDULER.md: `RtConfig` described as plain
// data, so a Web Worker can rebuild it from a message.
//
// `RtConfig` holds live objects a worker cannot be sent: a terrain sampler
// (functions), a seeded generator (state), a route planner and a passability
// check (functions). `RtConfigSpec` holds what they are built FROM — ids,
// seeds, numbers, the ground choice — and `configFromSpec` builds them.
//
// The one thing that is not small data is the terrain raster: megabytes of
// land cover, fetched from Foundry. The spec names it (`rasterRid`); the
// loaded raster itself is passed beside the spec as a resource. A worker gets
// it as one transferable buffer (TerrainRaster.exportGrid / fromGrid) rather
// than fetching it again.

import { effectiveScope, type Scope } from "../scope";
import type { LatLng } from "../../lib/board";
import { proceduralTerrain, STANDARD_GROUND } from "../../lib/proceduralTerrain";
import { rasterTerrain } from "../../lib/rasterTerrain";
import { rasterRoutePlanner, type RasterRouter } from "../../lib/routePlan";
import { mobilityClassFor, type TerrainFlagSource } from "../../lib/terrainAdapter";
import { createRng } from "../../rules/dice";
import { HOUSE_V1, type RuleSet } from "../../rules/ruleset";
import type { Template } from "../behaviour/types";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig, RtTiming } from "./types";

export const RT_CONFIG_SPEC_VERSION = 1;

/** Every ruleset a spec may name, by id. */
export const RULESETS: Readonly<Record<string, RuleSet>> = { [HOUSE_V1.id]: HOUSE_V1 };

/** The ruleset with this id. Throws for an unknown id rather than playing under the wrong rules. */
export function rulesetById(id: string): RuleSet {
  const ruleset = RULESETS[id];
  if (!ruleset) throw new Error(`Unknown ruleset "${id}" (known: ${Object.keys(RULESETS).join(", ")})`);
  return ruleset;
}

/**
 * The ground a game is fought on.
 *
 * - "generated": generated land cover and relief from `seed`; no raster.
 * - "raster+relief": the raster's real land cover over relief generated from `seed`.
 * - "raster": the raster's land cover, flat.
 *
 * With a raster, moves are routed by A* on it and its water is impassable;
 * without one, by the engine's bearing planner. That is the whole of the
 * planner's configuration, so it needs no field of its own.
 */
export interface RtGroundSpec {
  source: "generated" | "raster+relief" | "raster";
  /** Seed for generated land cover or relief. */
  seed: string;
  /** The board's centre: generated ground is laid out around it. */
  origin: LatLng;
  /** The dataset holding the cost raster. Required unless `source` is "generated". */
  rasterRid?: string;
}

/**
 * Timing as data. `reactionS` is left out: it is a formula (reaction time by
 * troop quality), fixed by the rules so replays are exact, and always
 * `DEFAULT_TIMING.reactionS`. `configFromSpec` puts it back.
 */
export type RtTimingSpec = Omit<RtTiming, "reactionS">;

/** A config's timing as data. Throws if its reaction time is not the standard one, which a spec cannot carry. */
export function timingSpec(timing: RtTiming): RtTimingSpec {
  if (timing.reactionS !== DEFAULT_TIMING.reactionS) {
    throw new Error("A custom reactionS cannot be described as data; only DEFAULT_TIMING.reactionS is supported");
  }
  const data: Partial<RtTiming> = { ...timing };
  delete data.reactionS;
  return data as RtTimingSpec;
}

/** Everything an `RtConfig` is built from, as plain data: JSON- and structured-clone-safe. */
export interface RtConfigSpec {
  version: typeof RT_CONFIG_SPEC_VERSION;
  rulesetId: string;
  /** The dice seed. The same seed and the same answers give the same game. */
  seed: string;
  /** Dice already rolled: resumes the generator mid-game. Default 0. */
  rngCursor?: number;
  timing: RtTimingSpec;
  comms?: RtConfig["comms"];
  ground: RtGroundSpec;
  behaviours?: Record<string, string>;
  behaviourTemplates?: Template[];
  /** What the game covers (scope.ts). Checked, and made runnable, by `configFromSpec`. */
  scope?: Scope;
}

/** A loaded raster: where each flag is, A* routes over it, and (optionally) what is passable. */
export type RtRaster = TerrainFlagSource & RasterRouter;

/** What a spec cannot carry: the loaded raster, when the ground uses one. */
export interface RtConfigResources {
  raster?: RtRaster | null;
}

/** The live terrain sampler, planner and passability check for a ground spec. */
export function groundFromSpec(
  ground: RtGroundSpec,
  raster: RtRaster | null | undefined,
): Pick<RtConfig, "terrain" | "planner" | "isPassable"> {
  if (ground.source === "generated") {
    return { terrain: proceduralTerrain({ ...STANDARD_GROUND, seed: ground.seed, origin: ground.origin }) };
  }
  if (!raster) {
    throw new Error(
      `Ground "${ground.source}" needs the terrain raster${ground.rasterRid ? ` (${ground.rasterRid})` : ""}: load it and pass it as resources.raster`,
    );
  }
  const relief = ground.source === "raster+relief" ? { ...STANDARD_GROUND, seed: ground.seed, origin: ground.origin } : null;
  const check = raster.isPassable;
  return {
    terrain: rasterTerrain(raster, { relief }),
    planner: rasterRoutePlanner(raster, mobilityClassFor),
    isPassable: check ? (point: LatLng) => check.call(raster, point.lat, point.lng) : undefined,
  };
}

/** Build the live config a spec describes. On the main thread or in a worker, the same game. */
export function configFromSpec(spec: RtConfigSpec, resources: RtConfigResources = {}): RtConfig {
  if (spec.version !== RT_CONFIG_SPEC_VERSION) {
    throw new Error(`Unsupported config spec version ${String(spec.version)} (expected ${RT_CONFIG_SPEC_VERSION})`);
  }
  return {
    ruleset: rulesetById(spec.rulesetId),
    rng: createRng(spec.seed, spec.rngCursor ?? 0),
    timing: { ...spec.timing, reactionS: DEFAULT_TIMING.reactionS },
    ...groundFromSpec(spec.ground, resources.raster),
    ...(spec.comms ? { comms: spec.comms } : {}),
    ...(spec.behaviours ? { behaviours: spec.behaviours } : {}),
    ...(spec.behaviourTemplates ? { behaviourTemplates: spec.behaviourTemplates } : {}),
    ...(spec.scope ? { scope: runnableScope(spec.scope) } : {}),
  };
}

/** The scope as the engine will run it; a scope it cannot run is refused, with every reason. */
function runnableScope(scope: Scope): Scope {
  const check = effectiveScope(scope);
  if (check.errors.length) throw new Error(`This scope cannot be played yet: ${check.errors.join(" ")}`);
  return check.scope;
}
