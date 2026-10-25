// ── realtime/world/geo.ts ────────────────────────────────────────────────
// Space for the world layer (MULTI_DOMAIN_PLAN.md §4.2): three dimensions,
// line of sight between two heights at any range, the radar horizon, and
// moving things that are fast enough to pass a target inside one second.

import { bearingDeg, distanceM, type LatLng } from "../../lib/board";
import type { TerrainSampler } from "../../lib/lineOfSight";
import { offsetBy, towards } from "../engine/geometry";

export { bearingDeg, distanceM, offsetBy, towards };

/** Radar (and radio) horizon between two heights, m: d ≈ 4.12·(√h₁ + √h₂) km, the 4/3-Earth approximation. */
export function horizonM(h1M: number, h2M: number): number {
  return 4120 * (Math.sqrt(Math.max(0, h1M)) + Math.sqrt(Math.max(0, h2M)));
}

/** Ground height, treating ground the terrain does not cover as sea level. */
export function groundAt(terrain: TerrainSampler, at: LatLng): number {
  const h = terrain.groundHeightM(at);
  return Number.isFinite(h) ? h : 0;
}

/**
 * Can a point at `fromAltM` (above sea level) see one at `toAltM`? Earth's
 * curvature by the radar horizon, then the terrain in between, sampled.
 * Heights are above sea level; a ground unit passes its ground height plus
 * its eye or mast.
 */
export function seesBetween(terrain: TerrainSampler, from: LatLng, fromAltM: number, to: LatLng, toAltM: number, samples = 24): boolean {
  const range = distanceM(from, to);
  if (range < 1) return true;
  const g1 = groundAt(terrain, from);
  const g2 = groundAt(terrain, to);
  if (range > horizonM(Math.max(1, fromAltM - Math.min(g1, 0)), Math.max(1, toAltM - Math.min(g2, 0)))) return false;
  // Above about 2 km the terrain of a 10 km board cannot block; the horizon has decided.
  if (fromAltM > 2000 && toAltM > 2000) return true;
  for (let i = 1; i < samples; i++) {
    const f = i / samples;
    const p = { lat: from.lat + (to.lat - from.lat) * f, lng: from.lng + (to.lng - from.lng) * f };
    const line = fromAltM + (toAltM - fromAltM) * f;
    // The bulge of the Earth between them, at this point, with 4/3 refraction.
    const bulge = (f * (1 - f) * range * range) / (2 * 8_500_000);
    if (groundAt(terrain, p) + bulge > line) return false;
  }
  return true;
}

/** One second's move towards a point at a speed, in 3D: where it ends, and whether it got there. */
export function step3d(at: LatLng, altM: number, to: LatLng, toAltM: number, speedMs: number, dtS: number): { at: LatLng; altM: number; arrived: boolean } {
  const flat = distanceM(at, to);
  const climb = toAltM - altM;
  const total = Math.hypot(flat, climb);
  const go = speedMs * dtS;
  if (total <= go || total < 1) return { at: { ...to }, altM: toAltM, arrived: true };
  const f = go / total;
  return { at: towards(at, to, flat * f), altM: altM + climb * f, arrived: false };
}

/**
 * Closest approach within one step: a mover going from `a0` to `a1` while
 * its target goes from `b0` to `b1` (both straight, both over one step).
 * Returns the least separation, m, so a 1.5 km/s interceptor cannot tunnel
 * through its target between two ticks.
 */
export function closestApproachM(
  a0: { at: LatLng; altM: number },
  a1: { at: LatLng; altM: number },
  b0: { at: LatLng; altM: number },
  b1: { at: LatLng; altM: number },
): number {
  // Local flat frame around a0, metres east and north.
  const origin = a0.at;
  const xy = (p: LatLng) => {
    const north = (p.lat - origin.lat) * 111_320;
    const east = (p.lng - origin.lng) * 111_320 * Math.cos((origin.lat * Math.PI) / 180);
    return { x: east, y: north };
  };
  const pa0 = { ...xy(a0.at), z: a0.altM };
  const pa1 = { ...xy(a1.at), z: a1.altM };
  const pb0 = { ...xy(b0.at), z: b0.altM };
  const pb1 = { ...xy(b1.at), z: b1.altM };
  // Relative position and velocity of b with respect to a over the step.
  const r = { x: pb0.x - pa0.x, y: pb0.y - pa0.y, z: pb0.z - pa0.z };
  const v = { x: pb1.x - pb0.x - (pa1.x - pa0.x), y: pb1.y - pb0.y - (pa1.y - pa0.y), z: pb1.z - pb0.z - (pa1.z - pa0.z) };
  const vv = v.x * v.x + v.y * v.y + v.z * v.z;
  const t = vv > 0 ? Math.max(0, Math.min(1, -(r.x * v.x + r.y * v.y + r.z * v.z) / vv)) : 0;
  return Math.hypot(r.x + v.x * t, r.y + v.y * t, r.z + v.z * t);
}

/** A point at a random offset from `at`, distributed as a circular error with this CEP (the 50% circle). */
export function scatter(at: LatLng, cepM: number, roll: (n: number) => number): LatLng {
  if (cepM <= 0) return at;
  const u = (roll(1_000_000) + 0.5) / 1_000_000;
  // Rayleigh: half the rounds land inside the CEP.
  const r = cepM * Math.sqrt(-Math.log(1 - u) / Math.LN2);
  const bearing = roll(360);
  return offsetBy(at, bearing, Math.min(r, cepM * 6));
}

/** Where a satellite in a circular orbit is over the Earth at time `t` (sub-satellite point). Two-body, no perturbations. */
export function subSatellitePoint(altKm: number, inclinationDeg: number, phaseDeg: number, t: number, raanDeg = 0): LatLng {
  const mu = 398_600.4418;
  const a = 6371 + altKm;
  const periodS = 2 * Math.PI * Math.sqrt((a * a * a) / mu);
  if (altKm > 30_000) {
    // Geostationary: fixed over its longitude.
    return { lat: 0, lng: raanDeg + phaseDeg };
  }
  const u = ((phaseDeg * Math.PI) / 180 + (2 * Math.PI * t) / periodS) % (2 * Math.PI);
  const i = (inclinationDeg * Math.PI) / 180;
  const lat = Math.asin(Math.sin(i) * Math.sin(u));
  // The Earth turns under the orbit at 360° per sidereal day.
  const lng = Math.atan2(Math.cos(i) * Math.sin(u), Math.cos(u)) + (raanDeg * Math.PI) / 180 - (2 * Math.PI * t) / 86_164;
  const deg = (x: number) => (x * 180) / Math.PI;
  return { lat: deg(lat), lng: ((deg(lng) + 540) % 360) - 180 };
}

export function orbitPeriodS(altKm: number): number {
  const a = 6371 + altKm;
  return 2 * Math.PI * Math.sqrt((a * a * a) / 398_600.4418);
}

/** Is `p` inside the polygon (lat/lng ring)? */
export function inPolygon(p: LatLng, ring: readonly LatLng[]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[i];
    const b = ring[j];
    if (a.lat > p.lat !== b.lat > p.lat && p.lng < ((b.lng - a.lng) * (p.lat - a.lat)) / (b.lat - a.lat) + a.lng) inside = !inside;
  }
  return inside;
}

/** Which side of a polyline (a phase line) a point is on: +1 left of its direction, −1 right. */
export function sideOfLine(p: LatLng, line: readonly LatLng[]): number {
  let best = Infinity;
  let sign = 1;
  for (let i = 0; i + 1 < line.length; i++) {
    const a = line[i];
    const b = line[i + 1];
    const cross = (b.lng - a.lng) * (p.lat - a.lat) - (b.lat - a.lat) * (p.lng - a.lng);
    const d = Math.min(distanceM(p, a), distanceM(p, b));
    if (d < best) {
      best = d;
      sign = cross >= 0 ? 1 : -1;
    }
  }
  return sign;
}
