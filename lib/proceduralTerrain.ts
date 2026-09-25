// ── bgws/lib/proceduralTerrain.ts ──────────────────────────────────────────
// Ground with hills and woods on it, generated from a seed.
//
// WHY THIS EXISTS ALONGSIDE THE REAL DEM
// --------------------------------------
// `samplerFromFlags` is the production path: real elevation, real land cover,
// fetched in the browser. It cannot run in a test or a batch — it needs the
// network and a tile cache — and the harness runs thousands of games offline.
//
// So the harness got `flatTerrain()`, and flat ground turned out to disable a
// third of the rulebook. Nothing is ever in cover, no sight line is ever
// broken by a ridge, and `targetInCover`, `defenderInCover` and every
// elevation effect are unreachable by construction. A sweep on flat ground
// reports those rules as having no effect, which is true and useless.
//
// This is not a substitute for the DEM. It is ground that is not flat, is the
// same every time for a given seed, and needs nothing to run — which is what
// a repeatable experiment requires.
//
// DELIBERATELY NOT A FRACTAL. Value noise on two octaves, because the point
// is hills and woodland at the scale a 3 km sight line cares about, and a
// prettier generator would be a bigger thing to trust.

import type { LatLng } from "./board";
import type { TerrainSampler } from "./lineOfSight";
import type { TerrainClass } from "./movement";

/** Metres per noise cell. 400 m gives hills a sight line can hide behind. */
const ELEVATION_CELL_M = 400;

/** Woodland is patchier than terrain. */
const COVER_CELL_M = 250;

/** Wet ground comes in bigger pieces than woodland. A lake is not a copse. */
const WET_CELL_M = 600;

const METRES_PER_DEGREE_LAT = 111_320;

/** Deterministic hash to a unit interval. No state, so sampling is pure. */
function hash2(x: number, y: number, seed: number): number {
  let h = Math.imul(x | 0, 0x27d4eb2d) ^ Math.imul(y | 0, 0x85ebca6b) ^ Math.imul(seed, 0xc2b2ae35);
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d);
  h = Math.imul(h ^ (h >>> 12), 0x297a2d39);
  h ^= h >>> 15;
  // >>> 0 keeps it unsigned; the divisor maps it to [0, 1).
  return (h >>> 0) / 4_294_967_296;
}

/** Smoothstep, so cells blend into hills rather than steps. */
function smooth(t: number): number {
  return t * t * (3 - 2 * t);
}

function valueNoise(x: number, y: number, seed: number): number {
  const x0 = Math.floor(x);
  const y0 = Math.floor(y);
  const fx = smooth(x - x0);
  const fy = smooth(y - y0);

  const a = hash2(x0, y0, seed);
  const b = hash2(x0 + 1, y0, seed);
  const c = hash2(x0, y0 + 1, seed);
  const d = hash2(x0 + 1, y0 + 1, seed);

  return a * (1 - fx) * (1 - fy) + b * fx * (1 - fy) + c * (1 - fx) * fy + d * fx * fy;
}

export interface ProceduralTerrainOptions {
  /** Anything; the same string gives the same ground every time. */
  seed?: string;
  /** Peak-to-trough elevation. 60 m is rolling, not mountainous. */
  reliefM?: number;
  /**
   * Share of the map under woodland, 0 to 1.
   *
   * 0.3 is enough that a sight line across 2 km usually clips some, without
   * making the whole board a forest — which would be as uninformative as
   * making it all flat, just in the other direction.
   */
  woodFraction?: number;
  /**
   * Share of the map under wet ground — marsh at the fringe, water at the
   * core — on its own noise channel, 0 to 1.
   *
   * ⚠ WHY THIS HAD TO EXIST: NOTHING ON THIS GROUND COULD STOP ANYTHING.
   *
   * The allowance rule distinguishes SLOW ground from IMPASSABLE ground, and
   * this repo's own speed table makes marsh and water impassable to wheeled
   * vehicles and a crawl for tracks and boots. Generating only open ground and
   * woods meant the `blocked` branch of that rule could not be reached by any
   * game played on this ground: a declared rule that no sequence of play could
   * exercise, which is the exact failure the wiring guard exists to catch —
   * one level below where the guard looks, because this is a table rather than
   * a modifier.
   *
   * Defaults to 0, so every ground defined before this stays exactly as it
   * was. STANDARD_GROUND_V2 turns it on.
   */
  wetFraction?: number;
  /** Origin the metre grid is measured from. */
  origin?: LatLng;
}

function seedNumber(seed: string): number {
  let h = 2_166_136_261;
  for (let i = 0; i < seed.length; i += 1) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16_777_619);
  }
  return h >>> 0;
}

/**
 * A terrain sampler with hills and woods, identical for a given seed.
 *
 * Pure and synchronous, so it satisfies the TerrainSampler contract and can
 * be used anywhere flatTerrain() can.
 */
export function proceduralTerrain(options: ProceduralTerrainOptions = {}): TerrainSampler {
  const seed = seedNumber(options.seed ?? "bgws");
  const reliefM = options.reliefM ?? 60;
  const woodFraction = options.woodFraction ?? 0.3;
  const wetFraction = options.wetFraction ?? 0;
  const origin = options.origin ?? { lat: 54.2, lng: 18.6 };

  const metresPerDegreeLng = METRES_PER_DEGREE_LAT * Math.cos((origin.lat * Math.PI) / 180);

  const toMetres = (point: LatLng): { east: number; north: number } => ({
    east: (point.lng - origin.lng) * metresPerDegreeLng,
    north: (point.lat - origin.lat) * METRES_PER_DEGREE_LAT,
  });

  return {
    groundHeightM(point) {
      const { east, north } = toMetres(point);
      // Two octaves: broad ridges plus smaller undulation. More would be
      // decoration; one would give sight lines a single smooth dome.
      const coarse = valueNoise(east / ELEVATION_CELL_M, north / ELEVATION_CELL_M, seed);
      const fine = valueNoise(
        east / (ELEVATION_CELL_M / 3),
        north / (ELEVATION_CELL_M / 3),
        seed ^ 0x9e3779b9,
      );
      return (coarse * 0.75 + fine * 0.25) * reliefM;
    },

    classify(point) {
      const { east, north } = toMetres(point);

      // Wet ground first, because a marsh with trees on it is still a marsh
      // as far as getting a vehicle across it is concerned. Own noise channel
      // and a coarser cell: bogs and lakes come bigger than copses, and a
      // 100 m puddle would be noise a 100 m route sample could not resolve.
      if (wetFraction > 0) {
        const w = valueNoise(east / WET_CELL_M, north / WET_CELL_M, seed ^ 0x1b873593);
        if (w > 1 - wetFraction * 0.3) return "water" as TerrainClass;
        if (w > 1 - wetFraction) return "marsh" as TerrainClass;
      }

      const n = valueNoise(east / COVER_CELL_M, north / COVER_CELL_M, seed ^ 0x51ed270b);
      // Thickest woodland in the middle of a patch, lighter at the edges,
      // open outside — so cover has a gradient rather than a hard boundary.
      if (n > 1 - woodFraction * 0.35) return "woodsThick" as TerrainClass;
      if (n > 1 - woodFraction) return "woodsLight" as TerrainClass;
      return "open" as TerrainClass;
    },
  };
}

/**
 * THE STANDARD GROUND. Named and versioned, because it decides outcomes.
 *
 * Terrain is not scenery here — it changed the winner in a third to a half of
 * all games and changed at least one decision in EVERY game. A batch result
 * is meaningless without knowing which ground it was fought over, so the
 * ground gets a version like a ruleset does.
 *
 * These values were swept (scripts/bgwsTerrainTune.ts), not chosen. Gentler
 * relief and lighter woodland than looks realistic, for a reason: at 30%
 * woodland both sides sit in cover unable to see each other and games run to
 * the turn limit, which measures nothing. 20 m of relief and 10% woodland
 * keeps terrain decisive while games still resolve.
 */
export const STANDARD_GROUND_V1: ProceduralTerrainOptions = {
  seed: "baltic-v1",
  reliefM: 20,
  woodFraction: 0.1,
};

/**
 * THE STANDARD GROUND, v2: the same ground with water in it.
 *
 * ⚠ V1 COULD NOT STOP ANYTHING, AND THAT MADE A WHOLE RULE UNMEASURABLE.
 *
 * Sampled, v1 comes out 97% open, 2% light woods, under 1% thick. Every Move
 * Type crosses all three. So when the movement allowance was finally wired
 * into option generation, the sweep priced it at ZERO — 0 decisions changed in
 * 360 games — not because the rule does nothing but because this ground asks
 * nothing of it. A rule that cannot bind is a rule that cannot be measured,
 * and the sweep's "no effect" verdict has to keep meaning what it says.
 *
 * v2 adds wet ground at `wetFraction` 0.2, which samples as 6% marsh and 1%
 * water — impassable to wheeled vehicles, a crawl for tracks and boots, and
 * big enough (600 m cells) that going round is a decision rather than a
 * rounding error.
 *
 * SWEPT, NOT CHOSEN. Eight candidate grounds × six force lists × 40 seeds,
 * comparing terrainMovement off against on:
 *
 *   baltic-v1-wet   decisions 125/240  outcomes 51/240  turns 8.3-14.8  ← this
 *   baltic-v4-wet   decisions 123/240  outcomes 47/240  turns 9.9-19.4
 *   baltic-v1       decisions  83/240  outcomes 32/240  turns 10.5-16.9
 *   baltic-v3-wet   decisions  74/240  outcomes 30/240  turns 9.9-12.6
 *   baltic-v2-wet   decisions   1/240  outcomes  0/240  — blobs miss the axes
 *
 * Chosen for the strongest effect with the shortest games. Re-measured on the
 * chosen ground at 100 seeds: terrainMovement changes a decision in 100 of 100
 * games and the WINNER in 58 of 100 on advance-to-contact-v1, mean game length
 * 8.9-14.9 turns, at most 2 of 100 drawn.
 *
 * ⚠ KNOWN BIAS, DECLARED, AND SINCE MEASURED PROPERLY. Per-side win rates on
 * this ground are lopsided — symmetric-control-v1 runs 27/72 to red — and that
 * is TERRAIN, not rules. Swap the deployments and the same list runs 54/46 the
 * other way; across three lists and both orientations the rules come out blue
 * 294, red 298. One approach is simply wetter and more overlooked than the
 * other, which is what real ground is like.
 *
 * Two consequences, both load-bearing:
 *   - A win rate on this ground is never by itself a statement about a
 *     commander or a rule. Play both orientations, as the commander trial and
 *     the balance guard in rules/calibration.test.ts both now do.
 *   - The per-list figures in reports/module-impact-*.md are single-orientation
 *     and therefore carry this bias. The MODULE comparisons in those reports do
 *     not: both arms are played on the same ground, so it cancels.
 *
 * The seed is DIFFERENT from v1's rather than reusing it, so that a replay's
 * recorded groundSeed still identifies exactly one ground.
 */
export const STANDARD_GROUND_V2: ProceduralTerrainOptions = {
  seed: "baltic-v1-wet",
  reliefM: 20,
  woodFraction: 0.1,
  wetFraction: 0.2,
};

/**
 * The ground a batch gets when it does not say otherwise.
 *
 * Aliased rather than spelled out at every call site, because the version that
 * matters is the one recorded in a result's provenance, and forty call sites
 * that each name a version is forty places to forget to bump.
 */
export const STANDARD_GROUND: ProceduralTerrainOptions = STANDARD_GROUND_V2;

/** Terrain classes that count as cover for the fire and sighting modifiers. */
const COVER_CLASSES: ReadonlySet<string> = new Set(["woodsLight", "woodsThick", "urban"]);

/**
 * Is this point in cover?
 *
 * Shared by the sighting and fire resolvers through the turn loop, so a unit
 * that is hard to see is also hard to hit — which is the same fact about the
 * same wood, and would be a bug if the two disagreed.
 */
export function inCover(terrain: TerrainSampler, point: LatLng): boolean {
  return COVER_CLASSES.has(terrain.classify(point));
}
