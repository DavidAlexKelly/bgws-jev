// ── bgws/lib/rasterTerrain.ts ──────────────────────────────────────────────
// Playing BGWS on the real ground, from the same raster Scenario Planner uses.
//
// WHAT WAS ACTUALLY MISSING
// -------------------------
// Nothing here is new capability. `terrainAdapter.samplerFromFlags` has always
// turned a flag source into a TerrainSampler, `TerrainRaster` has always held
// the flags, and `useTerrainRaster` has always loaded it for the planner. The
// game never called any of it: App.tsx used `flatTerrain()` with a comment
// saying this was the line that would change, and Play.tsx used generated
// ground. Same failure as the movement allowance, one layer up.
//
// THREE THINGS THAT ARE TRUE AND HAVE TO BE SAID OUT LOUD
// ------------------------------------------------------
// 1. THE RASTER HAS NO ELEVATION. It is a cost raster: land cover only. Line
//    of sight over real ground would therefore be decided entirely by woods
//    and built-up areas, with no ridges at all — and flat ground was measured
//    to disable a third of the rulebook. So relief can be supplied separately,
//    and when it is, it is GENERATED. Real cover, invented hills; the caller
//    is told which is which and the UI says so.
// 2. COVERAGE IS NOT UNIVERSAL. The Kaliningrad raster does not cover the
//    BGWS default board near Gdansk. Off the edge the raster answers "unknown"
//    rather than "open", and a game played on unknown ground is a game played
//    on a lie, so the board is moved to the data rather than the other way
//    round.
// 3. THE CLASS MAPPING IS COARSER THAN THE GAME. The raster has one Forest
//    where BGWS has light and thick woods, and no urban class at all. That
//    translation lives in terrainAdapter and is not repeated here.

import { proceduralTerrain, type ProceduralTerrainOptions } from "./proceduralTerrain";
import { samplerFromFlags, type TerrainFlagSource } from "./terrainAdapter";
import type { TerrainSampler } from "./lineOfSight";
import type { LatLng } from "./board";
import { BOARD_SIZE_M } from "./board";

/** Where the ground under a game came from. Shown to the player, not inferred. */
export type TerrainSource =
  /** Generated from a seed. Offline, repeatable, and not a real place. */
  | "generated"
  /** Real land cover, with generated relief because no DEM is available. */
  | "raster+relief"
  /** Real land cover, flat. Honest, and it turns off every elevation rule. */
  | "raster"
  /**
   * Real land cover AND real elevation, from the Offline World DEM.
   *
   * The first option in which nothing about the ground is invented. Every
   * other source either generates the hills or has none.
   */
  | "raster+dem";

export interface RasterTerrainOptions {
  /**
   * Generated relief to lay under the real land cover, or null for flat.
   *
   * NOT REAL AND NOT PRETENDING TO BE. It exists because the raster carries no
   * elevation and flat ground makes cover, defilade and every elevation
   * modifier unreachable. Delete this the day a DEM is available and pass its
   * sampler instead — the seam is one argument wide, deliberately.
   */
  relief?: ProceduralTerrainOptions | null;
  /**
   * Real bare-earth height in metres, or NaN where the DEM has no coverage.
   *
   * ⚠ THE DAY ANTICIPATED ABOVE. This is the seam the `relief` comment
   * promised, used rather than widened: a height function, injected, with no
   * mention of a DEM, a dataset or a decoder anywhere in this module. What
   * satisfies it in the app is @acc/decho-elevation's `heightAtLoaded`; what
   * satisfies it in the tests is a closure over a literal.
   *
   * NaN IS PART OF THE CONTRACT, not a failure. `lineOfSight` already reports
   * `noCoverage` for a non-finite height, and the DEM answers NaN for any cell
   * that is not resident — ocean, or simply not fetched yet. A sampler that
   * substituted zero would put a board at sea level and call it flat ground.
   *
   * Takes precedence over `relief`: real heights and invented ones should
   * never be mixed, and if both are supplied the caller has asked for both by
   * accident.
   */
  heightAt?: ((point: LatLng) => number) | null;
}

/**
 * A TerrainSampler backed by the real raster.
 *
 * Takes a flag source rather than a TerrainRaster so it can be tested without
 * a dataset, a network or a browser — the same reason terrainAdapter injects
 * its flag source.
 */
export function rasterTerrain(
  flags: TerrainFlagSource,
  options: RasterTerrainOptions = {},
): TerrainSampler {
  if (options.heightAt) return samplerFromFlags(flags, options.heightAt);
  const relief = options.relief ? proceduralTerrain(options.relief) : null;
  return samplerFromFlags(flags, relief ? (point: LatLng) => relief.groundHeightM(point) : undefined);
}

export interface RasterBounds {
  minLat: number;
  minLon: number;
  maxLat: number;
  maxLon: number;
}

/** The middle of the covered area — the only place a board certainly fits. */
export function centreOf(bounds: RasterBounds): LatLng {
  return {
    lat: (bounds.minLat + bounds.maxLat) / 2,
    lng: (bounds.minLon + bounds.maxLon) / 2,
  };
}

const METRES_PER_DEGREE_LAT = 111_320;

/**
 * Does the raster cover a whole board centred here?
 *
 * Checked at the CORNERS, not the centre: a board whose middle is inside the
 * data and whose flank is off the edge would report every cell out there as
 * unknown, and unknown silently reads as open ground once it reaches the game.
 * That is the failure this function exists to make impossible.
 */
export function coversBoard(
  flags: TerrainFlagSource,
  centre: LatLng,
  sizeM: number = BOARD_SIZE_M,
): boolean {
  const half = sizeM / 2;
  const dLat = half / METRES_PER_DEGREE_LAT;
  const dLng = half / (METRES_PER_DEGREE_LAT * Math.cos((centre.lat * Math.PI) / 180));

  for (const lat of [centre.lat - dLat, centre.lat, centre.lat + dLat]) {
    for (const lng of [centre.lng - dLng, centre.lng, centre.lng + dLng]) {
      if (flags.flagAt(lat, lng) === null) return false;
    }
  }
  return true;
}

/** A one-line description of what the game is being fought over. */
export function describeTerrainSource(
  source: TerrainSource,
  label: string,
  groundSeed: string,
): string {
  switch (source) {
    case "raster+dem":
      // The only line here that does not have to apologise for something.
      return `real land cover (${label}) and real elevation (Offline World DEM)`;
    case "raster+relief":
      return `real land cover (${label}); relief generated from "${groundSeed}" — the raster has no DEM`;
    case "raster":
      return `real land cover (${label}); flat — the raster has no DEM, so no ridge blocks a sight line`;
    default:
      return `generated ground, seed "${groundSeed}" — repeatable, and not a real place`;
  }
}
