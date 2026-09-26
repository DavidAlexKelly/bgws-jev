// ── bgws/realtime/engine/geometry.ts ───────────────────────────────────────
// Small spatial helpers for the real-time engine: offsets, and the search for
// positions worth moving to (cover, overwatch, out of sight, a flank).

import { bearingDeg, bearingDeltaDeg, distanceM, type LatLng } from "../../lib/board";
import { lineOfSight } from "../../lib/lineOfSight";
import { inCover } from "../../lib/proceduralTerrain";
import type { ForceElement } from "../../lib/state";
import type { RtConfig } from "./types";

export const COMPASS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

export function compass(from: LatLng, to: LatLng): string {
  return COMPASS[Math.round(bearingDeg(from, to) / 45) % 8];
}

export function offsetBy(from: LatLng, bearing: number, metres: number): LatLng {
  const rad = (bearing * Math.PI) / 180;
  return {
    lat: from.lat + (Math.cos(rad) * metres) / 111_320,
    lng: from.lng + (Math.sin(rad) * metres) / (111_320 * Math.cos((from.lat * Math.PI) / 180)),
  };
}

/** A point `metres` along the line from `from` towards `to`, or `to` if closer. */
export function towards(from: LatLng, to: LatLng, metres: number): LatLng {
  const total = distanceM(from, to);
  if (total <= metres || total === 0) return { ...to };
  const f = metres / total;
  return { lat: from.lat + (to.lat - from.lat) * f, lng: from.lng + (to.lng - from.lng) * f };
}

/** Metres per turn this element may cover on the ground at `at`. Zero is impassable. */
export function allowanceAt(fe: ForceElement, at: LatLng, config: RtConfig): number {
  if (config.isPassable && !config.isPassable(at)) return 0;
  const terrain = config.terrain.classify(at);
  return config.ruleset.movement[fe.moveType]?.[terrain] ?? 0;
}

export interface Position {
  purpose: "cover" | "overwatch" | "withdraw" | "flank";
  at: LatLng;
  summary: string;
}

/**
 * Positions worth moving to, judged from what this side knows.
 *
 * The best of 24 points (three rings, eight bearings) for each purpose, and
 * only points the element can actually stand on. Same idea as the turn
 * game's terrain-aware moves, written for this engine so neither depends on
 * the other.
 */
export function positionsFor(
  fe: ForceElement,
  known: readonly ForceElement[],
  config: RtConfig,
): Position[] {
  const reach = Math.max(0, ...fe.capabilities.map((c) => c.maxRangeM));
  const sees = (from: LatLng) =>
    known.filter(
      (enemy) =>
        distanceM(from, enemy.position) <= reach &&
        lineOfSight(config.terrain, { from, to: enemy.position }).visible,
    );
  const seenBy = (at: LatLng) =>
    known.filter(
      (enemy) =>
        distanceM(enemy.position, at) <= 3000 &&
        lineOfSight(config.terrain, { from: enemy.position, to: at }).visible,
    ).length;
  const nearest = (at: LatLng) =>
    known.length ? Math.min(...known.map((enemy) => distanceM(at, enemy.position))) : Infinity;

  const points = [300, 700, 1200]
    .flatMap((ring) => Array.from({ length: 8 }, (_, i) => offsetBy(fe.position, i * 45, ring)))
    .filter((point) => allowanceAt(fe, point, config) > 0);

  const describe = (at: LatLng) =>
    `${Math.round(distanceM(fe.position, at))} m ${compass(fe.position, at)}`;
  const out: Position[] = [];

  if (!inCover(config.terrain, fe.position)) {
    const cover = points
      .filter((point) => inCover(config.terrain, point))
      .sort((a, b) => distanceM(fe.position, a) - distanceM(fe.position, b))[0];
    if (cover) {
      out.push({
        purpose: "cover",
        at: cover,
        summary: `move ${describe(cover)} into cover (${config.terrain.classify(cover)})`,
      });
    }
  }

  if (known.length === 0) return out;

  const scored = points.map((point) => ({
    point,
    sees: sees(point),
    cover: inCover(config.terrain, point) ? 1 : 0,
    seenBy: seenBy(point),
    nearest: nearest(point),
  }));

  const overwatch = scored
    .filter((one) => one.sees.length > 0)
    .sort((a, b) => b.sees.length - a.sees.length || b.cover - a.cover || a.seenBy - b.seenBy)[0];
  if (overwatch) {
    out.push({
      purpose: "overwatch",
      at: overwatch.point,
      summary:
        `move ${describe(overwatch.point)} to overwatch` +
        `${overwatch.cover ? " from cover" : ""}, seeing ${overwatch.sees.map((e) => e.id).join(", ")}`,
    });
  }

  const here = nearest(fe.position);
  const withdraw = scored
    .filter((one) => one.nearest > here + 200)
    .sort((a, b) => a.seenBy - b.seenBy || b.cover - a.cover || b.nearest - a.nearest)[0];
  if (withdraw) {
    out.push({
      purpose: "withdraw",
      at: withdraw.point,
      summary:
        `pull back ${describe(withdraw.point)}` +
        `${withdraw.seenBy === 0 ? ", out of known sight" : `, seen by ${withdraw.seenBy}`}`,
    });
  }

  const flank = scored
    .map((one) => ({
      ...one,
      flanked: one.sees.filter(
        (enemy) =>
          enemy.facing != null &&
          bearingDeltaDeg(enemy.facing, bearingDeg(enemy.position, one.point)) >
            config.ruleset.frontArcDeg,
      ),
    }))
    .filter((one) => one.flanked.length > 0)
    .sort((a, b) => b.flanked.length - a.flanked.length || b.cover - a.cover)[0];
  if (flank) {
    out.push({
      purpose: "flank",
      at: flank.point,
      summary: `move ${describe(flank.point)} onto the flank of ${flank.flanked.map((e) => e.id).join(", ")}`,
    });
  }

  return out;
}

// ── Hull-down, from the elevation data ─────────────────────────────────────
//
// The raster has no vegetation height or buildings, so cover from them cannot
// be real. GROUND defilade can: the DEM says whether a crest between a
// vehicle and the enemy hides its hull while its turret can still see and
// shoot over it. That is what hull-down is.

/** DECLARED. Heights above ground of a vehicle's turret (sights, gun) and of its hull top. */
const TURRET_M = 2.4;
const HULL_M = 1.2;
/** The crest must be close in front of the vehicle to be ITS hull-down position, not a distant ridge. */
const CREST_WITHIN_M = 250;

/**
 * Is a vehicle at `at` hull-down against an enemy at `from`? The enemy can
 * see its turret but the ground, close in front of it, hides its hull.
 */
export function hullDownAgainst(at: LatLng, from: LatLng, config: RtConfig): boolean {
  const turret = lineOfSight(config.terrain, { from, to: at, targetHeightM: TURRET_M });
  if (!turret.visible) return false;
  const hull = lineOfSight(config.terrain, { from, to: at, targetHeightM: HULL_M, sampleSpacingM: 25 });
  return !hull.visible && hull.reason === "ground" && hull.blockedAt != null && distanceM(hull.blockedAt, at) <= CREST_WITHIN_M;
}

/** The nearest hull-down position against `threat` within `radius`, if the ground offers one. */
export function hullDownSpot(fe: ForceElement, threat: LatLng, config: RtConfig, radius = 200): LatLng | null {
  if (hullDownAgainst(fe.position, threat, config)) return fe.position;
  const rings = [radius / 4, radius / 2, (3 * radius) / 4, radius];
  for (const ring of rings) {
    const found = Array.from({ length: 8 }, (_, i) => offsetBy(fe.position, i * 45, ring))
      .filter((point) => allowanceAt(fe, point, config) > 0)
      .filter((point) => hullDownAgainst(point, threat, config))
      .sort((a, b) => distanceM(a, threat) - distanceM(b, threat))[0];
    if (found) return found;
  }
  return null;
}

// ── Speed, per platform ────────────────────────────────────────────────────

/** DECLARED. The stat-card speed the ruleset's allowance table corresponds to, by move type. */
const BASELINE_KMH: Record<string, number> = { T: 60, W: 90 };

/**
 * How much faster or slower than the ground-class table this platform is,
 * from its stat-card speed (L7 `statcard_speed_kmh`). 1 when the source is
 * silent, so nothing changes for platforms without the figure.
 */
export function platformSpeedFactor(fe: ForceElement): number {
  const base = BASELINE_KMH[fe.moveType];
  if (fe.speedKmh == null || base == null) return 1;
  return Math.max(0.6, Math.min(1.4, fe.speedKmh / base));
}

/** DECLARED. Uphill slowing, for a vehicle of 20 hp/t; a weaker one slows more. */
const SLOPE_PENALTY = 6;

/**
 * Uphill costs speed, more for an under-powered vehicle (L7 `hp_per_tonne`).
 * From the DEM between here and the next step; downhill and flat cost nothing.
 */
export function slopeFactor(fe: ForceElement, from: LatLng, to: LatLng, config: RtConfig): number {
  const run = distanceM(from, to);
  if (run <= 0) return 1;
  const rise = config.terrain.groundHeightM(to) - config.terrain.groundHeightM(from);
  if (!Number.isFinite(rise) || rise <= 0) return 1;
  const power = (fe.hpPerTonne ?? 20) / 20;
  return 1 / (1 + ((rise / run) * SLOPE_PENALTY) / Math.max(0.3, power));
}
