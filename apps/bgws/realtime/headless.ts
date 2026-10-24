// ── realtime/headless.ts ─────────────────────────────────────────────────
// The real-time game without a screen (MULTI_DOMAIN_PLAN.md W0 step 1 and
// its "done when"): play a scenario file, or replay a game recorded in the
// browser, in Node — and get the same event stream, to the byte.
//
// A GAME RECORD is everything a game started from: the config as data
// (`RtConfigSpec`), the state at the first tick (forces placed, opening orders
// given), and any orders issued while it was paused, with the time each was
// issued. The browser can download one for any game; `playRecord` replays it
// and fingerprints the event stream (fingerprint.ts). The browser shows the
// fingerprint of the game it played; the two agree, or something differs.
//
// What can be replayed: games decided by the rules. A game where Jev or the
// commander decided depends on answers from a language model, which are not
// in the record; it replays with the rules instead and will differ — the
// record says which it was. Ground from the Foundry raster needs that raster;
// generated ground needs nothing.

import { DEFAULT_ORIGIN, toGameStateFromPlaced, type PlacedElement } from "../lib/forceBuilder";
import type { Side } from "../lib/state";
import { HOUSE_V1 } from "../rules/ruleset";
import { configFromSpec, RT_CONFIG_SPEC_VERSION, timingSpec, type RtConfigSpec, type RtGroundSpec, type RtRaster } from "./engine/configSpec";
import { createRealtimeState } from "./engine/engine";
import { EventFingerprint, fromJson, toJson } from "./engine/fingerprint";
import { applyOrders, heuristicOrders } from "./engine/orders";
import type { RtLogEntry } from "./engine/runner";
import { InThreadRunner } from "./engine/runnerHandle";
import { DEFAULT_TIMING } from "./engine/timing";
import type { RtConfig, RtState, UnitOrders } from "./engine/types";
import type { RealtimeScenario } from "./scenario";
import type { Scope } from "./scope";

export const GAME_RECORD_FORMAT = "bgws.realtime.game";
export const GAME_RECORD_VERSION = 1;

/** Orders issued while the game was paused, and when. */
export interface IssuedOrders {
  time: number;
  orders: Record<string, UnitOrders>;
  plan?: { side: Side; text: string };
}

export interface GameRecord {
  format: typeof GAME_RECORD_FORMAT;
  version: typeof GAME_RECORD_VERSION;
  name: string;
  recordedAt: string;
  spec: RtConfigSpec;
  /** The state at the first tick: forces placed, opening orders given. */
  initial: RtState;
  issued: IssuedOrders[];
  /** Who made the units' decisions. Only "rules" games replay exactly. */
  deciders: "rules" | "jev";
  /** What the browser's run of it came to, if it has finished or been saved mid-game. */
  played?: { fingerprint: string; entries: number; time: number };
}

/** The settings a game is built from, wherever they come from (setup screen or scenario file). */
export interface GameSettings {
  gameSeed: string;
  /** The seed's suffix: "" for the first run, something fresh for each run after. */
  runSalt?: string;
  breakpointPct: number;
  timeLimitMin: number;
  comms: RtConfig["comms"];
  ground: RtGroundSpec;
  scope?: Scope;
  behaviours?: Record<string, string>;
  behaviourTemplates?: RtConfigSpec["behaviourTemplates"];
}

/** The config as data for a game: the ONE place this mapping lives, for the browser and for Node. */
export function gameSpec(settings: GameSettings): RtConfigSpec {
  return {
    version: RT_CONFIG_SPEC_VERSION,
    rulesetId: HOUSE_V1.id,
    seed: `${settings.gameSeed}:realtime${settings.runSalt ?? ""}`,
    timing: timingSpec({ ...DEFAULT_TIMING, breakpoint: settings.breakpointPct / 100, maxDurationS: settings.timeLimitMin * 60 }),
    comms: settings.comms,
    ground: settings.ground,
    ...(settings.scope ? { scope: { ...settings.scope, durationMin: settings.timeLimitMin } } : {}),
    ...(settings.behaviours ? { behaviours: settings.behaviours } : {}),
    ...(settings.behaviourTemplates ? { behaviourTemplates: settings.behaviourTemplates } : {}),
  };
}

/**
 * A scenario file's config as data. Generated ground is placed where the
 * setup screen places it with no raster; raster ground needs `origin`, the
 * centre the browser chose from the raster's bounds.
 */
export function scenarioSpec(scenario: RealtimeScenario, raster?: { origin: { lat: number; lng: number } }): RtConfigSpec {
  const usesRaster = scenario.ground.choice !== "generated";
  if (usesRaster && !raster) {
    throw new Error(`"${scenario.name}" is fought on the Foundry raster ("${scenario.ground.choice}"); playing it headless needs that raster.`);
  }
  return gameSpec({
    gameSeed: scenario.seed,
    breakpointPct: scenario.rules.breakpointPct,
    timeLimitMin: scenario.scope.durationMin,
    comms: scenario.rules.comms,
    ground: { source: scenario.ground.choice, seed: scenario.ground.seed, origin: raster?.origin ?? DEFAULT_ORIGIN },
    scope: scenario.scope,
    behaviours: scenario.behaviours,
    behaviourTemplates: scenario.behaviourTemplates,
  });
}

/** Forces placed and the simple opening orders given (the setup screen's "Simple orders"). */
export function openingState(forces: PlacedElement[], gameSeed: string, config: RtConfig): RtState {
  let state = createRealtimeState(toGameStateFromPlaced(forces, HOUSE_V1, { gameId: gameSeed }));
  for (const side of ["blue", "red"] as const) state = applyOrders(state, heuristicOrders(state, side), config);
  return state;
}

/** A scenario file as a game record: simple opening orders, the rules deciding. */
export function recordFromScenario(scenario: RealtimeScenario): GameRecord {
  const spec = scenarioSpec(scenario);
  return {
    format: GAME_RECORD_FORMAT,
    version: GAME_RECORD_VERSION,
    name: scenario.name,
    recordedAt: new Date(0).toISOString(),
    spec,
    initial: openingState(scenario.forces, scenario.seed, configFromSpec(spec)),
    issued: [],
    deciders: "rules",
  };
}

export const encodeRecord = (record: GameRecord): string => toJson(record);

/** Read a game record. */
export function decodeRecord(text: string): { record?: GameRecord; errors: string[] } {
  let raw: unknown;
  try {
    raw = fromJson(text);
  } catch {
    return { errors: ["Not JSON."] };
  }
  const r = raw as Partial<GameRecord> | null;
  if (!r || r.format !== GAME_RECORD_FORMAT) return { errors: [`Not a game record (format is "${String(r?.format)}").`] };
  if (r.version !== GAME_RECORD_VERSION) return { errors: [`Game record version ${String(r.version)} is not supported.`] };
  const errors: string[] = [];
  if (!r.spec || r.spec.version !== RT_CONFIG_SPEC_VERSION) errors.push("The record has no usable config (spec).");
  if (!r.initial || typeof r.initial.time !== "number") errors.push("The record has no starting state.");
  if (!Array.isArray(r.issued)) errors.push('"issued" must be a list.');
  return errors.length ? { errors } : { record: r as GameRecord, errors };
}

export interface PlayResult {
  state: RtState;
  fingerprint: string;
  entries: number;
  time: number;
}

/**
 * Play a record to its end (or to `untilS`), the rules deciding, and
 * fingerprint the event stream. `chunkS` is how far each advance goes; the
 * game does not depend on it (the runner waits in simulated time).
 */
export async function playRecord(
  record: GameRecord,
  options: { raster?: RtRaster | null; untilS?: number; chunkS?: number; onEntry?: (entry: RtLogEntry) => void } = {},
): Promise<PlayResult> {
  const needsRaster = record.spec.ground.source !== "generated";
  if (needsRaster && !options.raster) throw new Error("This game was fought on the Foundry raster; replaying it needs that raster.");
  const config = configFromSpec(record.spec, { raster: options.raster ?? null });
  const fingerprint = new EventFingerprint();
  const runner = new InThreadRunner(record.initial, config, {
    onEntry: (entry) => {
      fingerprint.add(entry);
      options.onEntry?.(entry);
    },
  });
  const issued = [...record.issued].sort((a, b) => a.time - b.time);
  const until = options.untilS ?? Infinity;
  const chunk = options.chunkS ?? 300;
  let next = 0;
  while (!runner.state.over && runner.state.time < until) {
    while (next < issued.length && issued[next].time <= runner.state.time) {
      await runner.issueOrders(issued[next].orders, issued[next].plan);
      next++;
    }
    const stop = Math.min(until, next < issued.length ? issued[next].time : Infinity);
    await runner.advance(Math.min(chunk, stop - runner.state.time));
  }
  return { state: runner.state, ...fingerprint.value, time: runner.state.time };
}
