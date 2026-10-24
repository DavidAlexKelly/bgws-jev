// ── realtime/catalogue/weapons.ts ────────────────────────────────────────
// Generic weapons, shared by the platform records that mount them (W14).
// One entry per kind of weapon, not per real system. Costs and kill
// probabilities are synthetic unless the research gives them; where it does,
// the record's `sources` says so ("weapons.<id>.costUsd").

import type { WeaponSpec } from "./schema";

const w = (spec: WeaponSpec): WeaponSpec => spec;

export const WEAPONS = {
  // ── Land, direct fire (the land engine resolves these itself; listed for completeness) ──
  tankGun: w({
    id: "tankGun", kind: "directFire", minRangeM: 0, maxRangeM: 3000, salvo: 1, magazine: 40, reloadS: 8, flight: "none", speedMs: 0,
    guidance: "none", pk: 0.6, cepM: 0, lethalRadiusM: 0, penetrationMm: 600, topAttack: false, targets: ["land"], family: "tank120", costUsd: 8000,
  }),
  atgm: w({
    id: "atgm", kind: "atgm", minRangeM: 65, maxRangeM: 4000, salvo: 1, magazine: 6, reloadS: 30, flight: "pursuit", speedMs: 180,
    guidance: "ir", pk: 0.7, cepM: 1, lethalRadiusM: 3, penetrationMm: 800, topAttack: true, targets: ["land"], family: "atgm", costUsd: 180_000,
  }),

  // ── Indirect fire: rounds in flight are entities ──
  mortar120: w({
    id: "mortar120", kind: "mortar", minRangeM: 500, maxRangeM: 7000, salvo: 2, magazine: 60, reloadS: 10, flight: "ballistic", speedMs: 200,
    guidance: "none", pk: 0.3, cepM: 60, lethalRadiusM: 40, penetrationMm: 0, topAttack: true, targets: ["land"], family: "mortar120", costUsd: 1500,
  }),
  howitzer155: w({
    id: "howitzer155", kind: "howitzer", minRangeM: 3000, maxRangeM: 24_000, salvo: 4, magazine: 40, reloadS: 12, flight: "ballistic", speedMs: 450,
    guidance: "none", pk: 0.35, cepM: 120, lethalRadiusM: 50, penetrationMm: 0, topAttack: true, targets: ["land"], family: "artillery155", costUsd: 3000,
  }),
  precision155: w({
    id: "precision155", kind: "howitzer", minRangeM: 6000, maxRangeM: 40_000, salvo: 1, magazine: 8, reloadS: 20, flight: "glide", speedMs: 400,
    guidance: "gnss", pk: 0.6, cepM: 4, lethalRadiusM: 50, penetrationMm: 0, topAttack: true, targets: ["land"], family: "artillery155", costUsd: 100_000,
  }),
  rocketGmlrs: w({
    id: "rocketGmlrs", kind: "rocket", minRangeM: 15_000, maxRangeM: 80_000, salvo: 2, magazine: 12, reloadS: 30, flight: "ballistic", speedMs: 900,
    guidance: "gnss", pk: 0.7, cepM: 5, lethalRadiusM: 60, penetrationMm: 0, topAttack: true, targets: ["land"], family: "rocket", costUsd: 160_000,
  }),

  // ── Air defence ──
  samLong: w({
    id: "samLong", kind: "samLong", minRangeM: 3000, maxRangeM: 160_000, salvo: 2, magazine: 16, reloadS: 10, flight: "pursuit", speedMs: 1500,
    guidance: "activeRadar", pk: 0.65, cepM: 0, lethalRadiusM: 20, penetrationMm: 0, topAttack: false, targets: ["air", "munition"], family: "samLong", costUsd: 4_200_000,
  }),
  samMedium: w({
    id: "samMedium", kind: "samMedium", minRangeM: 1500, maxRangeM: 40_000, salvo: 2, magazine: 8, reloadS: 8, flight: "pursuit", speedMs: 1000,
    guidance: "radar", pk: 0.6, cepM: 0, lethalRadiusM: 15, penetrationMm: 0, topAttack: false, targets: ["air", "munition"], family: "samMedium", costUsd: 1_000_000,
  }),
  samShort: w({
    id: "samShort", kind: "samShort", minRangeM: 300, maxRangeM: 8000, salvo: 1, magazine: 8, reloadS: 6, flight: "pursuit", speedMs: 700,
    guidance: "ir", pk: 0.5, cepM: 0, lethalRadiusM: 5, penetrationMm: 0, topAttack: false, targets: ["air", "munition"], family: "samShort", costUsd: 150_000,
  }),
  gunAd: w({
    id: "gunAd", kind: "gunAd", minRangeM: 0, maxRangeM: 3000, salvo: 1, magazine: 40, reloadS: 3, flight: "none", speedMs: 0,
    guidance: "radar", pk: 0.25, cepM: 0, lethalRadiusM: 2, penetrationMm: 0, topAttack: false, targets: ["air", "munition"], family: "cannon30", costUsd: 2000,
  }),
  shotgun: w({
    id: "shotgun", kind: "shotgun", minRangeM: 0, maxRangeM: 60, salvo: 1, magazine: 50, reloadS: 2, flight: "none", speedMs: 0,
    guidance: "none", pk: 0.2, cepM: 0, lethalRadiusM: 1, penetrationMm: 0, topAttack: false, targets: ["air"], family: "smallArms", costUsd: 5,
  }),
  interceptorDrone: w({
    id: "interceptorDrone", kind: "interceptorDrone", minRangeM: 0, maxRangeM: 15_000, salvo: 1, magazine: 6, reloadS: 60, flight: "pursuit", speedMs: 70,
    guidance: "operator", pk: 0.55, cepM: 0, lethalRadiusM: 3, penetrationMm: 0, topAttack: false, targets: ["air", "munition"], family: "drone", costUsd: 6000,
  }),

  // ── Air-launched ──
  aamBvr: w({
    id: "aamBvr", kind: "aam", minRangeM: 1000, maxRangeM: 100_000, salvo: 1, magazine: 4, reloadS: 5, flight: "pursuit", speedMs: 1200,
    guidance: "activeRadar", pk: 0.55, cepM: 0, lethalRadiusM: 12, penetrationMm: 0, topAttack: false, targets: ["air"], family: "aam", costUsd: 1_100_000,
  }),
  aamWvr: w({
    id: "aamWvr", kind: "aam", minRangeM: 300, maxRangeM: 15_000, salvo: 1, magazine: 2, reloadS: 3, flight: "pursuit", speedMs: 900,
    guidance: "ir", pk: 0.65, cepM: 0, lethalRadiusM: 8, penetrationMm: 0, topAttack: false, targets: ["air"], family: "aam", costUsd: 450_000,
  }),
  agm: w({
    id: "agm", kind: "agm", minRangeM: 500, maxRangeM: 8000, salvo: 1, magazine: 8, reloadS: 10, flight: "pursuit", speedMs: 400,
    guidance: "laser", pk: 0.75, cepM: 1, lethalRadiusM: 5, penetrationMm: 1000, topAttack: true, targets: ["land", "surface"], family: "agm", costUsd: 150_000,
  }),
  glideBomb: w({
    id: "glideBomb", kind: "glideBomb", minRangeM: 10_000, maxRangeM: 70_000, salvo: 1, magazine: 2, reloadS: 30, flight: "glide", speedMs: 250,
    guidance: "gnss", pk: 0.6, cepM: 10, lethalRadiusM: 80, penetrationMm: 0, topAttack: true, targets: ["land"], family: "bomb", costUsd: 30_000,
  }),
  arm: w({
    id: "arm", kind: "arm", minRangeM: 5000, maxRangeM: 120_000, salvo: 1, magazine: 2, reloadS: 10, flight: "pursuit", speedMs: 900,
    guidance: "antiRadiation", pk: 0.6, cepM: 5, lethalRadiusM: 15, penetrationMm: 0, topAttack: true, targets: ["land", "surface"], family: "missile", costUsd: 900_000,
  }),

  // ── Strike ──
  cruise: w({
    id: "cruise", kind: "cruise", minRangeM: 20_000, maxRangeM: 1_600_000, salvo: 1, magazine: 8, reloadS: 30, flight: "cruise", speedMs: 240,
    guidance: "gnss", pk: 0.8, cepM: 5, lethalRadiusM: 40, penetrationMm: 0, topAttack: true, targets: ["land", "surface"], family: "missile", costUsd: 2_000_000,
  }),
  ballistic: w({
    id: "ballistic", kind: "ballistic", minRangeM: 50_000, maxRangeM: 500_000, salvo: 1, magazine: 2, reloadS: 600, flight: "ballistic", speedMs: 1700,
    guidance: "inertial", pk: 0.7, cepM: 30, lethalRadiusM: 70, penetrationMm: 0, topAttack: true, targets: ["land", "surface"], family: "missile", costUsd: 3_000_000,
  }),
  owa: w({
    id: "owa", kind: "owa", minRangeM: 20_000, maxRangeM: 1_500_000, salvo: 1, magazine: 1, reloadS: 120, flight: "cruise", speedMs: 50,
    guidance: "gnss", pk: 0.5, cepM: 15, lethalRadiusM: 25, penetrationMm: 0, topAttack: true, targets: ["land"], family: "drone", costUsd: 40_000,
  }),

  // ── Drones as munitions ──
  fpv: w({
    id: "fpv", kind: "fpv", minRangeM: 0, maxRangeM: 15_000, salvo: 1, magazine: 12, reloadS: 120, flight: "pursuit", speedMs: 30,
    guidance: "operator", pk: 0.45, cepM: 0, lethalRadiusM: 4, penetrationMm: 300, topAttack: true, targets: ["land"], family: "drone", costUsd: 500,
  }),
  loitering: w({
    id: "loitering", kind: "loitering", minRangeM: 0, maxRangeM: 40_000, salvo: 1, magazine: 4, reloadS: 300, flight: "cruise", speedMs: 30,
    guidance: "operator", pk: 0.6, cepM: 0, lethalRadiusM: 6, penetrationMm: 500, topAttack: true, targets: ["land"], family: "drone", costUsd: 35_000,
  }),

  // ── Sea ──
  ascm: w({
    id: "ascm", kind: "ascm", minRangeM: 5000, maxRangeM: 250_000, salvo: 2, magazine: 8, reloadS: 20, flight: "seaSkim", speedMs: 280,
    guidance: "activeRadar", pk: 0.7, cepM: 0, lethalRadiusM: 0, penetrationMm: 0, topAttack: false, targets: ["surface"], family: "antiShip", costUsd: 1_500_000,
  }),
  torpedo: w({
    id: "torpedo", kind: "torpedo", minRangeM: 500, maxRangeM: 40_000, salvo: 1, magazine: 12, reloadS: 120, flight: "underwater", speedMs: 25,
    guidance: "acoustic", pk: 0.7, cepM: 0, lethalRadiusM: 0, penetrationMm: 0, topAttack: false, targets: ["surface", "subsurface"], family: "torpedo", costUsd: 3_500_000,
  }),
  ciws: w({
    id: "ciws", kind: "ciws", minRangeM: 0, maxRangeM: 2000, salvo: 1, magazine: 30, reloadS: 2, flight: "none", speedMs: 0,
    guidance: "radar", pk: 0.35, cepM: 0, lethalRadiusM: 1, penetrationMm: 0, topAttack: false, targets: ["air", "munition"], family: "ciws", costUsd: 10_000,
  }),
  navalGun: w({
    id: "navalGun", kind: "howitzer", minRangeM: 1000, maxRangeM: 24_000, salvo: 2, magazine: 300, reloadS: 4, flight: "ballistic", speedMs: 700,
    guidance: "none", pk: 0.3, cepM: 80, lethalRadiusM: 30, penetrationMm: 0, topAttack: true, targets: ["land", "surface", "air"], family: "artillery155", costUsd: 2000,
  }),
} as const satisfies Record<string, WeaponSpec>;

export type WeaponId = keyof typeof WEAPONS;

export function weaponById(id: string): WeaponSpec | undefined {
  return (WEAPONS as Record<string, WeaponSpec>)[id];
}

/**
 * How much of a ship's or an installation's staying power one round takes,
 * in standard hits (a 200 kg warhead is 1). Synthetic, by warhead size.
 */
const STRUCTURAL_HITS: Partial<Record<WeaponSpec["kind"], number>> = {
  ascm: 1,
  torpedo: 2,
  cruise: 1.2,
  ballistic: 1.5,
  glideBomb: 1.2,
  agm: 0.25,
  arm: 0.3,
  fpv: 0.02,
  loitering: 0.06,
  owa: 0.3,
  howitzer: 0.05,
  mortar: 0.03,
  rocket: 0.15,
  samLong: 0.1,
  samMedium: 0.08,
  samShort: 0.04,
  ciws: 0.01,
  gunAd: 0.01,
  directFire: 0.03,
  atgm: 0.05,
};

export function structuralHits(weapon: WeaponSpec): number {
  return STRUCTURAL_HITS[weapon.kind] ?? 0.05;
}
