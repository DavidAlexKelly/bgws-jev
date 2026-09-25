// ── bgws/lib/lineOfSight.ts ────────────────────────────────────────────────
// Can this Force Element see that one?
//
// Unlike the resolution tables, line of sight IS fully specified in the Core
// Rulebook (2.1.14), so it can be implemented properly rather than stubbed:
//
//   · nothing is visible beyond 3,000 m, which is also the longest any weapon
//     may fire;
//   · steep hills, woods and urban areas block;
//   · a blocking FEATURE adds 20 m to the ground height beneath it — which is
//     exactly the correction a bare-earth DEM needs, since it models neither
//     trees nor buildings;
//   · an intervening counter at the same elevation blocks;
//   · higher ground sees over lower.
//
// THE ELEVATION SOURCE IS INJECTED. This module imports no DEM, no map and no
// package: a caller passes a sampler. @acc/decho-elevation's createDemSource()
// satisfies it in about ten lines, a flat-earth stub satisfies it in one, and
// the tests use the stub. That keeps the rule testable without a dataset and
// keeps this file honest about which parts are BGWS and which are terrain.

import { distanceM, LOS_CAP_M, type LatLng } from "./board";
import { interpolate } from "./movement";
import type { TerrainClass } from "./movement";

/** Height in metres that woods and buildings add to bare ground (BGWS 2.1.14). */
export const FEATURE_HEIGHT_M = 20;

/** Terrain whose features block a sight line rather than merely slowing a move. */
const BLOCKING_TERRAIN: ReadonlySet<TerrainClass> = new Set([
  "woodsLight",
  "woodsThick",
  "urban",
]);

/** Default observer/target height: a person or a vehicle commander's head. */
export const DEFAULT_EYE_HEIGHT_M = 2;

/**
 * How the caller supplies terrain. Both functions are synchronous by design:
 * an async sampler would make every range check in the engine async, so the
 * caller pre-loads the cells it needs (the DEM source caches whole 2° cells)
 * and answers from memory.
 */
export interface TerrainSampler {
  /** Bare-earth height in metres. NaN where there is no coverage. */
  groundHeightM(point: LatLng): number;
  /** Terrain class at a point, for the feature-height correction. */
  classify(point: LatLng): TerrainClass;
}

/** A counter that blocks by standing in the way (BGWS 2.1.14). */
export interface BlockingCounter {
  position: LatLng;
  /** Counters block within roughly their own footprint. */
  radiusM?: number;
}

export interface LineOfSightRequest {
  from: LatLng;
  to: LatLng;
  observerHeightM?: number;
  targetHeightM?: number;
  /** Other FEs on the map. The observer and target are excluded by position. */
  counters?: readonly BlockingCounter[];
  /** Sample spacing. 50 m over a 3 km line is 60 samples. */
  sampleSpacingM?: number;
}

export type BlockReason =
  | "clear"
  | "beyondRange"
  | "ground"
  | "feature"
  | "counter"
  | "noCoverage";

export interface LineOfSightResult {
  visible: boolean;
  reason: BlockReason;
  rangeM: number;
  /** Where the line was first blocked, when it was. */
  blockedAt?: LatLng;
  /** Metres by which the obstruction exceeded the sight line. */
  obstructionM?: number;
}

const DEFAULT_SAMPLE_SPACING_M = 50;
const COUNTER_BLOCK_RADIUS_M = 60;

/**
 * Resolve a sight line.
 *
 * Earth curvature is deliberately NOT modelled. It matters from about 10 km —
 * 1.7 m of bulge at the middle of a 10 km line — and BGWS stops at 3 km, where
 * the correction is under 20 cm and smaller than the DEM's own vertical error.
 * (@acc/decho-elevation's own lineOfSight does model it, for the longer lines
 * it is asked about; if this is ever swapped onto that call directly, the
 * answer changes by nothing at this range.)
 */
export function lineOfSight(
  terrain: TerrainSampler,
  request: LineOfSightRequest,
): LineOfSightResult {
  const { from, to } = request;
  const rangeM = distanceM(from, to);

  if (rangeM > LOS_CAP_M) {
    return { visible: false, reason: "beyondRange", rangeM };
  }

  const observerHeight = request.observerHeightM ?? DEFAULT_EYE_HEIGHT_M;
  const targetHeight = request.targetHeightM ?? DEFAULT_EYE_HEIGHT_M;

  const fromGround = terrain.groundHeightM(from);
  const toGround = terrain.groundHeightM(to);
  if (!Number.isFinite(fromGround) || !Number.isFinite(toGround)) {
    return { visible: false, reason: "noCoverage", rangeM };
  }

  const eyeHeight = fromGround + observerHeight;
  const targetTop = toGround + targetHeight;

  const spacing = request.sampleSpacingM ?? DEFAULT_SAMPLE_SPACING_M;
  const samples = Math.max(2, Math.ceil(rangeM / spacing));
  const counters = request.counters ?? [];

  for (let i = 1; i < samples; i++) {
    const fraction = i / samples;
    const point = interpolate(from, to, fraction);

    // Height of the sight line above sea level at this fraction along it.
    const sightLine = eyeHeight + (targetTop - eyeHeight) * fraction;

    const ground = terrain.groundHeightM(point);
    if (!Number.isFinite(ground)) {
      return { visible: false, reason: "noCoverage", rangeM, blockedAt: point };
    }

    if (ground > sightLine) {
      return {
        visible: false,
        reason: "ground",
        rangeM,
        blockedAt: point,
        obstructionM: round1(ground - sightLine),
      };
    }

    // The DEM is bare earth: trees and buildings are not in it, so the rule's
    // 20 m is added here rather than hoped for from the data.
    if (BLOCKING_TERRAIN.has(terrain.classify(point))) {
      const featureTop = ground + FEATURE_HEIGHT_M;
      if (featureTop > sightLine) {
        return {
          visible: false,
          reason: "feature",
          rangeM,
          blockedAt: point,
          obstructionM: round1(featureTop - sightLine),
        };
      }
    }

    for (const counter of counters) {
      const radius = counter.radiusM ?? COUNTER_BLOCK_RADIUS_M;
      if (distanceM(point, counter.position) > radius) continue;
      // A counter blocks only at its own elevation: an FE in a valley does not
      // mask a ridge line behind it.
      const counterGround = terrain.groundHeightM(counter.position);
      if (!Number.isFinite(counterGround)) continue;
      // `>=`, not `>`, and the difference is the whole rule: BGWS blocks on a
      // counter "at the same elevation", which on flat ground is exactly
      // equality. A strict comparison let two FEs see straight through a third
      // standing between them on a plain, which is the commonest case there is.
      if (counterGround + DEFAULT_EYE_HEIGHT_M >= sightLine) {
        return {
          visible: false,
          reason: "counter",
          rangeM,
          blockedAt: counter.position,
        };
      }
    }
  }

  return { visible: true, reason: "clear", rangeM };
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}

/**
 * A sampler for flat ground with no features.
 *
 * Not a toy: it is what the board falls back to before a DEM Resource is
 * granted, and it makes the geometry of every other rule testable on its own.
 * It reports clear line of sight everywhere inside 3 km, which is the correct
 * answer for flat, bare ground.
 */
export function flatTerrain(heightM = 0): TerrainSampler {
  return {
    groundHeightM: () => heightM,
    classify: () => "open",
  };
}
