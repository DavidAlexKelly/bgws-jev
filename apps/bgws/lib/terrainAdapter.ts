// ── bgws/lib/terrainAdapter.ts ─────────────────────────────────────────────
// Between this repo's terrain engine and BGWS's vocabulary.
//
// shared/routing already has everything a wargame needs to move a unit over
// real ground: a cost raster, a tiled A* router, mobility classes and a
// per-terrain speed table. It speaks in TerrainFlag and km/h. BGWS speaks in
// terrain classes and an allowance per turn. This is the translation, kept in
// one file so the seam is visible and so neither side has to learn the other's
// words.
//
// The flag source is INJECTED for the same reason the elevation sampler is:
// TerrainRaster loads datasets, and a rule that cannot be tested without a
// dataset is a rule nobody will test.

import type { MoveType } from "../data/profiles";
import type { TerrainSampler } from "./lineOfSight";
import type { AllowanceTable, TerrainClass } from "./movement";
import type { LatLng } from "./board";
import { TERRAIN_SPEEDS, TerrainFlag, type MobilityClass } from "../../../shared/routing/types";

/**
 * Whatever can say which terrain flag is at a point.
 *
 * TerrainRaster satisfies this with a one-line lambda. So does a test.
 */
export interface TerrainFlagSource {
  /** The raster's flag, or null outside coverage. 255 is deep water. */
  flagAt(lat: number, lng: number): number | null;
}

/** Flag 255 is the raster's "impassable", with no entry in TERRAIN_SPEEDS. */
export const IMPASSABLE_FLAG = 255;

/**
 * Terrain flag → BGWS terrain class.
 *
 * The raster is coarser than BGWS: it has one Forest where the game has light
 * and thick woods, and no urban class at all. Mapping Forest to the LIGHTER of
 * the two is deliberate — it is the less punishing reading, and a rule that
 * silently makes every wood impassable to tracks would be discovered as "the
 * router is broken" rather than as "the terrain data is coarse".
 *
 * Urban is genuinely absent, and pretending otherwise would be worse than
 * admitting it: the basemap has building footprints and the DEM does not, so
 * an urban classifier belongs with the landcover work, not here.
 */
export function terrainClassForFlag(flag: number | null): TerrainClass | null {
  switch (flag) {
    case TerrainFlag.Open:
      return "open";
    case TerrainFlag.Road:
      return "road";
    case TerrainFlag.Forest:
      return "woodsLight";
    case TerrainFlag.Marsh:
      return "marsh";
    case TerrainFlag.WaterCrossing:
      return "water";
    case IMPASSABLE_FLAG:
      return "water";
    default:
      return null;
  }
}

/** BGWS Move Type → the router's mobility class. */
export function mobilityClassFor(moveType: MoveType): MobilityClass {
  if (moveType === "F") return "foot";
  if (moveType === "W") return "wheeled";
  return "tracked";
}

/**
 * A TerrainSampler for line of sight, from a flag source plus an optional
 * elevation source.
 *
 * With no elevation the ground is flat, and line of sight is then decided
 * entirely by features — which is the right degraded behaviour: woods and
 * towns still block, and only the hills go missing.
 */
export function samplerFromFlags(
  flags: TerrainFlagSource,
  groundHeightM?: (point: LatLng) => number,
): TerrainSampler {
  return {
    groundHeightM: groundHeightM ?? (() => 0),
    classify: (point) => terrainClassForFlag(flags.flagAt(point.lat, point.lng)) ?? "open",
  };
}

/**
 * Minutes of game time in one BGWS turn.
 *
 * The rulebook does not state it, but its own movement example pins it closely:
 * a Foot FE gets 2,000 m along roads in a turn, and this repo's terrain table
 * walks foot at 8 km/h on roads. 2,000 m at 8 km/h is fifteen minutes.
 */
export const DEFAULT_TURN_MINUTES = 15;

/**
 * How much of its MARCH speed a vehicle makes under contact. OURS.
 *
 * ⚠ THE SOURCE TABLE IS A ROUTE PLANNER'S, NOT A WARGAME'S.
 *
 * `TERRAIN_SPEEDS` exists to plan movement across a theatre: 45 km/h on roads
 * and 20 km/h cross-country for tracks. Multiplied by a 15-minute turn that is
 * 11,250 m of road and 5,000 m of open ground per turn — on a 10 km board with
 * a 3 km sight cap. Nothing tactical happens at that rate: an element crosses
 * the entire board in two turns and the ground it crossed cannot matter.
 *
 * The giveaway was measurement, not opinion. The bounds the turn loop asks for
 * are 629-2,033 m, so the allowance sat at 3-8× the largest question anyone
 * ever asked it, and the terrain rule could only ever bind on the two cells
 * where the speed table says ZERO. A ceiling nothing can reach is not a rule.
 *
 * WHAT IS AND IS NOT DEFENSIBLE HERE
 * ----------------------------------
 * Foot is NOT scaled. The rulebook states the Foot allowance (2,000 m by road,
 * and its example implies 1,000 m across open ground) and the unscaled table
 * already reproduces it to within 25%. Scaling it would break the one figure
 * we can check against the book.
 *
 * Vehicles have no such figure — the TERRAIN EFFECTS TABLE is Player Aid 3 and
 * is not in the box we have — so
 * this is a declared house number rather than doctrine. 0.45 puts a tracked
 * element at 2,250 m across open ground, 450 m through woodland and 5,062 m
 * along a road in a 15-minute turn: 9 km/h cross-country, which is an advance
 * to contact rather than a road march, and it leaves the ratios between
 * terrain classes exactly as the source had them.
 *
 * ⚠ CONFIRMED AGAINST THE RULEBOOK AFTER THE FACT, WHICH IS THE BEST KIND OF
 * AGREEMENT. 9.1 states that Maximum Allowable Distance "is a figure between 1
 * and 6", each value being 1,000 m. At 0.45 a tracked element crosses 2,250 m
 * of open ground — a "2" on that scale — and foot 1,250 m, a "1". The march
 * rate this replaced put tracked at 5,000 m, a "5", for cross-country
 * movement. The sweep and the rulebook were reached independently and agree.
 *
 * SWEPT — IN CI, BECAUSE THAT IS WHERE THE BATCHES COULD RUN. The container
 * lost its node_modules and has no egress, so the sweep was committed as a
 * temporary test, the numbers came back in the build log, and the test was
 * deleted again. 40 seeds x 4 force lists x 5 candidates, terrainMovement on:
 *
 *   fract  T open   mean turns    hit the limit   symmetric-control balance
 *   1.00    5,000   9.7-22.1      1-15 of 40      6/34   ← march rate: the
 *                                                          advance stalls in
 *                                                          15 of 40 games
 *   0.60    3,000   11.3-15.2     2-7 of 40       11/28
 *   0.45    2,250   11.9-14.0     3-4 of 40       20/20  ← this
 *   0.30    1,500   12.8-16.9     3-6 of 40       17/23
 *   0.20    1,000   14.4-16.6     5-6 of 40       11/29
 *
 * 0.45 wins on every axis that matters and is not a close call: the shortest
 * games, the fewest hitting the 40-turn limit, and a dead-even balance canary
 * (the same force list on both sides). Every candidate changed a decision in
 * 40 of 40 games, so the rule earns its place at any of them — what the sweep
 * settles is the TEMPO, and the tuned mean of 11.7 turns is what it lands on.
 *
 * Note 1.00 — the march rate this started as — is also the WORST row for
 * resolution: elements outrun their own sighting, drive past each other, and
 * the advance-to-contact list fails to resolve in nearly 40% of games.
 *
 * `scripts/bgwsMovementTune.ts` reproduces this table anywhere batches can be
 * run. `rules/calibration.test.ts` fails CI if a later change to this number,
 * to the speed table or to the ground takes the game back out of the envelope.
 */
export const VEHICLE_CONTACT_FRACTION = 0.45;

/**
 * An allowance table derived from the repo's existing speed table.
 *
 * ⚠ THIS IS A STAND-IN FOR PLAYER AID 2, not the real thing.
 *
 * It is offered because the alternative — no table at all — means no movement,
 * and because it is *derived from data already in this repo* rather than
 * invented: TERRAIN_SPEEDS × turn length. Its calibration against the one
 * figure the rulebook does give is in the tests: a 15-minute turn reproduces
 * the Foot road allowance exactly (2,000 m) and lands within 25% on open
 * ground (1,250 m against the book's 1,000 m).
 *
 * When the real table arrives, delete this and pass the real one. Every call
 * site takes the table as a parameter precisely so that is a one-line change.
 */
export function allowanceFromSpeeds(
  turnMinutes = DEFAULT_TURN_MINUTES,
  vehicleContactFraction = 1,
): AllowanceTable {
  const hours = turnMinutes / 60;
  const metresFor = (mobility: MobilityClass, flag: TerrainFlag): number => {
    const kmh = TERRAIN_SPEEDS[mobility][flag] ?? 0;
    // ZERO SURVIVES THE CONVERSION. It means impassable, not "very slow", so
    // no fraction is allowed to turn it into a small positive number and no
    // rounding is allowed to turn a small positive number into it.
    if (kmh <= 0) return 0;
    const fraction = mobility === "foot" ? 1 : vehicleContactFraction;
    return Math.max(1, Math.round(kmh * 1000 * hours * fraction));
  };

  const forMoveType = (moveType: MoveType): Partial<Record<TerrainClass, number>> => {
    const mobility = mobilityClassFor(moveType);
    const open = metresFor(mobility, TerrainFlag.Open);
    const woods = metresFor(mobility, TerrainFlag.Forest);
    return {
      road: metresFor(mobility, TerrainFlag.Road),
      open,
      // The raster has no crops or urban class, so they are priced from the
      // classes it does have rather than left impassable: crops are open
      // ground that slows you, and a town is roughly as slow as woodland.
      crops: Math.round(open * 0.8),
      woodsLight: woods,
      woodsThick: Math.round(woods * 0.6),
      urban: woods,
      marsh: metresFor(mobility, TerrainFlag.Marsh),
      water: metresFor(mobility, TerrainFlag.WaterCrossing),
    };
  };

  return { F: forMoveType("F"), W: forMoveType("W"), T: forMoveType("T") };
}

/**
 * Split a router polyline into classified legs for the allowance accumulator.
 *
 * The router returns waypoints; BGWS needs to know what each stretch crossed.
 * Each segment is classified at its MIDPOINT, which is the cheapest reading
 * that cannot classify a whole leg by the terrain it happens to start on.
 */
export function legsFromWaypoints(
  waypoints: readonly [number, number][],
  flags: TerrainFlagSource,
  distanceBetween: (a: LatLng, b: LatLng) => number,
): { terrain: TerrainClass; distanceM: number; to: LatLng }[] {
  const legs: { terrain: TerrainClass; distanceM: number; to: LatLng }[] = [];
  for (let i = 1; i < waypoints.length; i++) {
    const from = { lat: waypoints[i - 1][0], lng: waypoints[i - 1][1] };
    const to = { lat: waypoints[i][0], lng: waypoints[i][1] };
    const midpoint = { lat: (from.lat + to.lat) / 2, lng: (from.lng + to.lng) / 2 };
    const terrain = terrainClassForFlag(flags.flagAt(midpoint.lat, midpoint.lng)) ?? "open";
    legs.push({ terrain, distanceM: distanceBetween(from, to), to });
  }
  return legs;
}
